//! The shared-guest owner process: the shell that runs [`OwnerCore`] against a
//! unix socket and one guest.
//!
//! The elector starts this process in a new session and a new PID namespace of
//! which it is init, so the kernel ends the guest's VMM with it. One thread
//! accepts connections, one thread per connection reads its request and watches
//! for the client going away, and one thread owns the guest; the main thread
//! serialises every event through the core and executes the effects it returns.
//!
//! The owner calls `waitpid` on nothing: it would steal the launcher's VMM wait
//! status.

#![allow(
    clippy::print_stderr,
    reason = "the owner's stderr is its log, redirected to a file by the elector"
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::guest_suite::{SharedBootRequest, SharedExecOutcome, SharedExecRequest, SharedGuest};

use super::owner_core::{AttachId, Effect, Event, OwnerCore, Verdict};
use super::shared::{
    Request, SHARED_GUEST_IDLE, TAG_ACKNOWLEDGED, TAG_STARTED, TAG_VERDICT, exec_bound, read_frame,
    verdict_text, write_frame,
};

/// How often the accepting thread and the guest thread look at their stop
/// conditions.
const POLL_SLICE: Duration = Duration::from_millis(20);
/// How often an idle guest thread checks that its VMM is still there.
const GUEST_WATCH_SLICE: Duration = Duration::from_millis(500);
/// How long a client has to send its request after connecting.
const REQUEST_BOUND: Duration = Duration::from_secs(10);
/// How long a write to a client may block.
const CLIENT_WRITE_BOUND: Duration = Duration::from_secs(30);
/// How long the owner waits for the guest thread to drop its guest on exit.
const GUEST_DROP_BOUND: Duration = Duration::from_secs(5);

/// What a thread tells the main thread.
enum Message {
    /// A client connected and sent its request.
    Incoming { id: AttachId, request: Request, stream: UnixStream },
    /// A client's connection ended.
    Gone(AttachId),
    /// The guest thread finished a command.
    Guest(GuestReport),
}

enum GuestCommand {
    Boot(Request),
    Exec(AttachId, Request),
    Discard,
    Stop,
}

enum GuestReport {
    Booted(Result<(Duration, Duration), String>),
    Exited { id: AttachId, status: i32, console: String },
    BoundElapsed { id: AttachId, console: String },
    Lost { console: String },
}

/// Runs the owner of the socket, group and profile named on the command line.
/// Never returns: the owner ends with `exit`.
pub fn run(arguments: &[String]) -> ! {
    let [socket, group, profile] = arguments else {
        eprintln!("usage: overdrive-shared-guest-owner <socket> <group> <profile>");
        std::process::exit(2);
    };
    let socket = PathBuf::from(socket);
    // A stale path of an owner that died is the only thing that can be here: the
    // election lock admits one starter at a time.
    let _ = std::fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("OWNER binding {}: {error}", socket.display());
            std::process::exit(3);
        }
    };
    let (messages, inbox) = mpsc::channel();
    let stop_accepting = Arc::new(AtomicBool::new(false));
    let acceptor = spawn_acceptor(listener, &messages, &stop_accepting);
    let (commands, guest_commands) = mpsc::channel();
    let guest_dropped = Arc::new(AtomicBool::new(false));
    spawn_guest_thread(
        group.clone(),
        profile.clone(),
        guest_commands,
        messages,
        Arc::clone(&guest_dropped),
    );
    eprintln!("OWNER serving {} for group {group} profile {profile}", socket.display());
    let mut owner = Owner {
        core: OwnerCore::new(),
        clients: BTreeMap::new(),
        commands,
        group: group.clone(),
        profile: profile.clone(),
        running_generation: 0,
        pending_console: String::new(),
        idle_since: Some(Instant::now()),
    };
    owner.serve(&inbox);
    // `Effect::Exit`: unlink the path first, then close the listener, so a
    // successor that binds after the close never has its socket unlinked here.
    let _ = std::fs::remove_file(&socket);
    stop_accepting.store(true, Ordering::SeqCst);
    let _ = acceptor.join();
    let _ = owner.commands.send(GuestCommand::Stop);
    let deadline = Instant::now() + GUEST_DROP_BOUND;
    while !guest_dropped.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(POLL_SLICE);
    }
    eprintln!("OWNER idle: exiting");
    std::process::exit(0);
}

/// A connected client the core knows.
struct Client {
    request: Request,
    stream: UnixStream,
}

struct Owner {
    core: OwnerCore,
    clients: BTreeMap<AttachId, Client>,
    commands: Sender<GuestCommand>,
    group: String,
    profile: String,
    /// The generation of the exec in flight, or of the last one.
    running_generation: u32,
    /// What the guest thread reported for the exec the core is about to answer.
    pending_console: String,
    idle_since: Option<Instant>,
}

impl Owner {
    /// Handles messages until the core exits.
    fn serve(&mut self, inbox: &mpsc::Receiver<Message>) {
        loop {
            let received = self.idle_since.map_or_else(
                || inbox.recv().map_err(|_| RecvTimeoutError::Disconnected),
                |since| {
                    inbox.recv_timeout(
                        (since + SHARED_GUEST_IDLE).saturating_duration_since(Instant::now()),
                    )
                },
            );
            let exited = match received {
                Ok(message) => self.handle(message),
                Err(RecvTimeoutError::Timeout) => self.apply(Event::IdleElapsed),
                Err(RecvTimeoutError::Disconnected) => true,
            };
            if exited {
                return;
            }
            self.idle_since = if self.is_idle() {
                self.idle_since.or_else(|| Some(Instant::now()))
            } else {
                None
            };
        }
    }

    fn is_idle(&self) -> bool {
        let observed = self.core.observe();
        observed.in_flight.is_none() && observed.queue.is_empty()
    }

    fn handle(&mut self, message: Message) -> bool {
        match message {
            Message::Incoming { id, request, mut stream } => {
                // Acknowledge before anything runs for the request; from here
                // the connection ending is owner loss for the client.
                if send(&mut stream, &[TAG_ACKNOWLEDGED]).is_err() {
                    return false;
                }
                self.clients.insert(id, Client { request, stream });
                self.apply(Event::Attached(id))
            }
            Message::Gone(id) => self.apply(Event::ClientGone(id)),
            Message::Guest(GuestReport::Booted(Ok((staging, boot)))) => {
                eprintln!(
                    "OVERDRIVE_GUEST_KIT_TIMING group={} shared=1 generation={} staging_ms={} boot_ms={}",
                    self.group,
                    self.core.observe().generation,
                    staging.as_millis(),
                    boot.as_millis(),
                );
                self.apply(Event::BootFinished(true))
            }
            Message::Guest(GuestReport::Booted(Err(detail))) => {
                eprintln!("OWNER boot failed: {detail}");
                self.pending_console = detail;
                self.apply(Event::BootFinished(false))
            }
            Message::Guest(GuestReport::Exited { id, status, console }) => {
                self.pending_console = console;
                self.apply(Event::ExecFinished(id, status))
            }
            Message::Guest(GuestReport::BoundElapsed { id, console }) => {
                self.pending_console = console;
                self.apply(Event::ExecBoundElapsed(id))
            }
            Message::Guest(GuestReport::Lost { console }) => {
                self.pending_console = console;
                self.apply(Event::GuestLost)
            }
        }
    }

    /// Feeds one event to the core and executes what it returns. True when the
    /// core asked the owner to exit.
    fn apply(&mut self, event: Event) -> bool {
        let generation_before = self.core.observe().generation;
        let mut exit = false;
        for effect in self.core.handle(event) {
            match effect {
                Effect::StartBoot => self.start_boot(),
                Effect::StartExec(id) => self.start_exec(id),
                Effect::Reply(id, verdict) => self.reply(id, &verdict, generation_before),
                Effect::DiscardGuest => {
                    let _ = self.commands.send(GuestCommand::Discard);
                }
                Effect::Exit => exit = true,
            }
        }
        self.pending_console.clear();
        exit
    }

    fn start_boot(&self) {
        let head = self
            .core
            .observe()
            .queue
            .first()
            .copied()
            .and_then(|id| self.clients.get(&id))
            .unwrap_or_else(|| unreachable!("a boot starts only with a client queued"));
        let _ = self.commands.send(GuestCommand::Boot(head.request.clone()));
    }

    fn start_exec(&mut self, id: AttachId) {
        let Some(client) = self.clients.get_mut(&id) else { return };
        self.running_generation = self.core.observe().generation;
        let _ = send(&mut client.stream, &[TAG_STARTED]);
        let _ = send(&mut client.stream, &self.running_generation.to_be_bytes());
        let _ = self.commands.send(GuestCommand::Exec(id, client.request.clone()));
    }

    fn reply(&mut self, id: AttachId, verdict: &Verdict, generation_before: u32) {
        let Some(mut client) = self.clients.remove(&id) else { return };
        let generation = if matches!(verdict, Verdict::BootFailed) {
            generation_before
        } else {
            self.running_generation
        };
        let text = verdict_text(verdict);
        let generation = generation.to_string();
        let sent = send(&mut client.stream, &[TAG_VERDICT]).and_then(|()| {
            write_frame(
                &mut client.stream,
                &[text.as_bytes(), generation.as_bytes(), self.pending_console.as_bytes()],
            )
        });
        if let Err(error) = sent {
            eprintln!("OWNER replying to client {id:?}: {error}");
        }
        let _ = client.stream.shutdown(Shutdown::Both);
    }
}

/// Writes `bytes` to a client, bounded so a client that never reads cannot hold
/// the owner.
fn send(stream: &mut UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    stream.set_write_timeout(Some(CLIENT_WRITE_BOUND))?;
    stream.write_all(bytes)
}

fn spawn_acceptor(
    listener: UnixListener,
    messages: &Sender<Message>,
    stop: &Arc<AtomicBool>,
) -> JoinHandle<()> {
    let messages = messages.clone();
    let stop = Arc::clone(stop);
    let next_id = Arc::new(AtomicU32::new(1));
    std::thread::spawn(move || {
        if listener.set_nonblocking(true).is_err() {
            return;
        }
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let id = AttachId::new(next_id.fetch_add(1, Ordering::SeqCst));
                    let messages = messages.clone();
                    std::thread::spawn(move || watch_connection(id, stream, &messages));
                }
                Err(_) => std::thread::sleep(POLL_SLICE),
            }
        }
    })
}

/// Reads a connection's request, hands it to the main thread, then reports when
/// the client goes away.
fn watch_connection(id: AttachId, stream: UnixStream, messages: &Sender<Message>) {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(REQUEST_BOUND)).is_err()
    {
        return;
    }
    let Ok(mut reader) = stream.try_clone() else { return };
    let Some(request) = read_frame(&mut reader).ok().and_then(Request::decode) else { return };
    if reader.set_read_timeout(None).is_err()
        || messages.send(Message::Incoming { id, request, stream }).is_err()
    {
        return;
    }
    let mut sink = [0_u8; 64];
    while matches!(reader.read(&mut sink), Ok(read) if read > 0) {}
    let _ = messages.send(Message::Gone(id));
}

fn spawn_guest_thread(
    group: String,
    profile: String,
    commands: mpsc::Receiver<GuestCommand>,
    reports: Sender<Message>,
    dropped: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut guest: Option<SharedGuest> = None;
        let report = |report| {
            let _ = reports.send(Message::Guest(report));
        };
        loop {
            match commands.recv_timeout(GUEST_WATCH_SLICE) {
                Ok(GuestCommand::Boot(request)) => {
                    guest = None;
                    match boot(&group, &profile, &request) {
                        Ok((booted, staging, boot)) => {
                            guest = Some(booted);
                            report(GuestReport::Booted(Ok((staging, boot))));
                        }
                        Err(detail) => report(GuestReport::Booted(Err(detail))),
                    }
                }
                Ok(GuestCommand::Exec(id, request)) => {
                    let Some(running) = guest.as_mut() else {
                        report(GuestReport::Lost { console: String::new() });
                        continue;
                    };
                    let outcome = exec(running, &group, &profile, &request);
                    match outcome {
                        SharedExecOutcome::Exited { status, console } => {
                            report(GuestReport::Exited { id, status, console });
                        }
                        SharedExecOutcome::BoundElapsed { console } => {
                            report(GuestReport::BoundElapsed { id, console });
                        }
                        SharedExecOutcome::Lost { console, detail } => {
                            guest = None;
                            report(GuestReport::Lost { console: format!("{console}\n{detail}") });
                        }
                    }
                }
                Ok(GuestCommand::Discard) => guest = None,
                Ok(GuestCommand::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {
                    if guest.as_ref().is_some_and(|running| running.has_exited(Duration::ZERO)) {
                        guest = None;
                        report(GuestReport::Lost {
                            console: "the guest's VMM exited while no test ran".to_owned(),
                        });
                    }
                }
            }
        }
        drop(guest);
        dropped.store(true, Ordering::SeqCst);
    });
}

fn boot(
    group: &str,
    profile: &str,
    request: &Request,
) -> Result<(SharedGuest, Duration, Duration), String> {
    // The group's staging reads the working directory the test runs from.
    std::env::set_current_dir(&request.working_directory)
        .map_err(|error| format!("entering {}: {error}", request.working_directory.display()))?;
    let program = request.program.to_string_lossy().into_owned();
    SharedGuest::boot(&SharedBootRequest {
        group,
        profile,
        program: &program,
        binary_id: &request.binary_id,
        environment: &request.environment,
        test_name: &request.test_name,
    })
}

fn exec(
    guest: &mut SharedGuest,
    group: &str,
    profile: &str,
    request: &Request,
) -> SharedExecOutcome {
    let program = request.program.to_string_lossy().into_owned();
    guest.exec(
        &SharedExecRequest {
            program: &program,
            environment: &request.environment,
            working_directory: Path::new(&request.working_directory),
            test_name: &request.test_name,
        },
        exec_bound(group, profile),
    )
}
