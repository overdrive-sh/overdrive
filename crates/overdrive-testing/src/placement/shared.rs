//! The shared-guest protocol: the scope key, the rendezvous paths, the wire
//! format and the client half (`attach_shared`). The owner half is the
//! `overdrive-shared-guest-owner` binary, which runs [`owner`](super::owner).
//!
//! A scope is one `shared_guest!` declaration of one test binary: its key is a
//! pure function of the nextest binary id, the declaring module path, the group
//! and the profile. The first test of a scope to arrive elects the scope's owner
//! process under an exclusive `flock`; later tests find its socket and attach.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the client returns the guest console to the test's own output and writes the timing line"
)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::guest_kit::SCRIPTED_BOOT_BOUND;
use crate::guest_suite::{RunInGuestError, shared_request_environment};
use crate::vm_fixture::{cargo_target_dir, workspace_root};

use super::owner_core::Verdict;
use super::{GuestPlacement, SharedEndpoint};

/// The owner binary `cargo xtask guest-kit-prepare` leaves under the cargo
/// target directory's `debug/`.
pub const OWNER_BINARY: &str = "overdrive-shared-guest-owner";

/// How long an owner stays up with nothing queued and nothing running.
pub const SHARED_GUEST_IDLE: Duration = Duration::from_secs(30);

/// The bound on one shared exec (the profile's per-test setup, the test body
/// and the profile's cleanup together): the cell bound of the groups' bodies.
pub fn exec_bound(group: &str, profile: &str) -> Duration {
    if profile == "nested-cohort" {
        Duration::from_secs(1_200)
    } else if profile.starts_with("nested") || group == "nested-boundary" {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(300)
    }
}

/// The scope of one declaration, as a stable file-name-safe key.
pub fn scope_key(binary_id: &str, declaring_module: &str, group: &str, profile: &str) -> String {
    // FNV-1a over the fields, each followed by a NUL so that moving a byte
    // between fields changes the key.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for field in [binary_id, declaring_module, group, profile] {
        for byte in field.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Where the prepared owner binary is.
pub fn owner_binary_path() -> PathBuf {
    cargo_target_dir(&workspace_root()).join("debug").join(OWNER_BINARY)
}

/// The node view the calling test runs in, as a file-name-safe token: the
/// identity of its cgroup namespace.
///
/// An owner lives in the cgroup of the test that elected it, and a test placed
/// as a native launcher runs in a cgroup of its own that the wrapper kills with
/// the test. Rendezvous is therefore per node view: a test never attaches to an
/// owner that a different node view's end would take away, and tests of the
/// run's own node view (every guest-placed test) share their scope.
fn node_view() -> String {
    fs::read_link("/proc/self/ns/cgroup")
        .map(|link| link.to_string_lossy().chars().filter(char::is_ascii_digit).collect::<String>())
        .unwrap_or_default()
}

/// The rendezvous and owner binary of `placement`'s scope in this run.
pub fn endpoint_for(placement: &GuestPlacement) -> Result<SharedEndpoint, String> {
    let directory = crate::guest_kit::shared_rendezvous_dir()
        .map_err(|error| format!("{error:?}"))?
        .join(node_view());
    let binary_id = std::env::var("NEXTEST_BINARY_ID").unwrap_or_default();
    let key =
        scope_key(&binary_id, placement.declaring_module, placement.group, placement.profile());
    Ok(SharedEndpoint {
        socket: directory.join(format!("{key}.sock")),
        lock: directory.join(format!("{key}.lock")),
        owner_binary: owner_binary_path(),
    })
}

// ----- wire format ---------------------------------------------------------

/// The owner acknowledged the request; nothing has run yet.
pub const TAG_ACKNOWLEDGED: u8 = b'A';
/// The test's exec started on this guest generation (a 4-byte big-endian
/// generation follows).
pub const TAG_STARTED: u8 = b'S';
/// The verdict frame follows.
pub const TAG_VERDICT: u8 = b'R';

const MAX_FRAME_FIELDS: u32 = 1 << 20;
const MAX_FRAME_FIELD_BYTES: u32 = 1 << 30;

/// Writes one frame: a field count, then each field as a length and its bytes.
pub fn write_frame(out: &mut impl Write, fields: &[&[u8]]) -> io::Result<()> {
    let too_large = || io::Error::new(io::ErrorKind::InvalidInput, "a frame field is too large");
    let mut buffer =
        Vec::with_capacity(4 + fields.iter().map(|field| 4 + field.len()).sum::<usize>());
    buffer.extend_from_slice(&u32::try_from(fields.len()).map_err(|_| too_large())?.to_be_bytes());
    for field in fields {
        buffer
            .extend_from_slice(&u32::try_from(field.len()).map_err(|_| too_large())?.to_be_bytes());
        buffer.extend_from_slice(field);
    }
    out.write_all(&buffer)
}

/// Reads one frame written by [`write_frame`].
pub fn read_frame(input: &mut impl Read) -> io::Result<Vec<Vec<u8>>> {
    let malformed = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let count = read_u32(input)?;
    if count > MAX_FRAME_FIELDS {
        return Err(malformed("a frame announces too many fields"));
    }
    let mut fields = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let length = read_u32(input)?;
        if length > MAX_FRAME_FIELD_BYTES {
            return Err(malformed("a frame field announces too many bytes"));
        }
        let mut field = vec![0_u8; length as usize];
        input.read_exact(&mut field)?;
        fields.push(field);
    }
    Ok(fields)
}

pub fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0_u8; 4];
    input.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

/// What one attaching test asks the owner to run.
#[derive(Clone)]
pub struct Request {
    pub test_name: String,
    pub working_directory: PathBuf,
    pub program: PathBuf,
    pub binary_id: String,
    pub environment: Vec<(String, String)>,
}

impl Request {
    /// The request of the running test.
    fn of_this_test(test_name: &str) -> Result<Self, String> {
        Ok(Self {
            test_name: test_name.to_owned(),
            working_directory: std::env::current_dir()
                .map_err(|error| format!("reading the test's working directory: {error}"))?,
            program: std::env::current_exe()
                .map_err(|error| format!("reading the test binary's path: {error}"))?,
            binary_id: std::env::var("NEXTEST_BINARY_ID").unwrap_or_default(),
            environment: shared_request_environment()?,
        })
    }

    pub fn encode(&self, out: &mut impl Write) -> io::Result<()> {
        let mut fields: Vec<&[u8]> = vec![
            self.test_name.as_bytes(),
            self.working_directory.as_os_str().as_bytes(),
            self.program.as_os_str().as_bytes(),
            self.binary_id.as_bytes(),
        ];
        for (key, value) in &self.environment {
            fields.push(key.as_bytes());
            fields.push(value.as_bytes());
        }
        write_frame(out, &fields)
    }

    pub fn decode(fields: Vec<Vec<u8>>) -> Option<Self> {
        let mut fields = fields.into_iter();
        let test_name = String::from_utf8(fields.next()?).ok()?;
        let working_directory = PathBuf::from(std::ffi::OsString::from_vec(fields.next()?));
        let program = PathBuf::from(std::ffi::OsString::from_vec(fields.next()?));
        let binary_id = String::from_utf8(fields.next()?).ok()?;
        let mut environment = Vec::new();
        while let Some(key) = fields.next() {
            let value = fields.next()?;
            environment.push((String::from_utf8(key).ok()?, String::from_utf8(value).ok()?));
        }
        Some(Self { test_name, working_directory, program, binary_id, environment })
    }
}

/// The wire text of a verdict.
pub fn verdict_text(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Exit(status) => format!("exit:{status}"),
        Verdict::BootFailed => "boot_failed".to_owned(),
        Verdict::GuestLost => "guest_lost".to_owned(),
        Verdict::ExecBound => "exec_bound".to_owned(),
    }
}

// ----- the client ----------------------------------------------------------

/// How many times a test attaches: the first attach and one re-attach after a
/// connection that ended before the owner acknowledged the request.
const ATTACH_ATTEMPTS: u32 = 2;

/// The test a failure names.
struct Names<'a> {
    group: &'a str,
    test: &'a str,
}

impl Names<'_> {
    fn not_prepared(&self, detail: String) -> RunInGuestError {
        RunInGuestError::LaneNotPrepared {
            group: self.group.to_owned(),
            test: self.test.to_owned(),
            detail,
        }
    }

    fn at_stage(&self, stage: &str, detail: &str) -> RunInGuestError {
        RunInGuestError::GuestExecution {
            group: self.group.to_owned(),
            test: self.test.to_owned(),
            detail: format!("stage {stage}: {detail}"),
        }
    }
}

/// Attaches to the owner of `endpoint`'s scope, electing one when none serves,
/// and runs `test_name` in the shared guest.
pub fn attach(
    endpoint: &SharedEndpoint,
    placement: &GuestPlacement,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    let names = Names { group: placement.group, test: test_name };
    if !endpoint.owner_binary.is_file() {
        return Err(names.not_prepared(format!(
            "the shared-guest owner binary {} is absent; run `cargo xtask guest-kit-prepare`",
            endpoint.owner_binary.display()
        )));
    }
    let request =
        Request::of_this_test(test_name).map_err(|detail| names.at_stage("owner", &detail))?;
    for _ in 0..ATTACH_ATTEMPTS {
        let mut stream = connect_or_elect(endpoint, placement)
            .map_err(|detail| names.at_stage("owner", &detail))?;
        if send_and_await_acknowledgement(&mut stream, &request) {
            return await_verdict(&names, &mut stream);
        }
    }
    Err(names
        .at_stage("owner", "the connection ended before the owner acknowledged the request, twice"))
}

/// Sends the request and reads the acknowledgement. False when the connection
/// ended first: the owner ran nothing for this request.
fn send_and_await_acknowledgement(stream: &mut UnixStream, request: &Request) -> bool {
    if request.encode(stream).is_err() {
        return false;
    }
    let mut tag = [0_u8; 1];
    stream.read_exact(&mut tag).is_ok() && tag[0] == TAG_ACKNOWLEDGED
}

/// Reads the owner's messages after the acknowledgement until the verdict.
/// A connection that ends from here on is owner loss.
fn await_verdict(names: &Names<'_>, stream: &mut UnixStream) -> Result<(), RunInGuestError> {
    let lost = |error: &dyn std::fmt::Display| {
        names
            .at_stage("owner", &format!("the shared-guest owner ended before the verdict: {error}"))
    };
    loop {
        let mut tag = [0_u8; 1];
        stream.read_exact(&mut tag).map_err(|error| lost(&error))?;
        match tag[0] {
            TAG_STARTED => {
                let generation = read_u32(stream).map_err(|error| lost(&error))?;
                eprintln!(
                    "OVERDRIVE_GUEST_KIT_TIMING group={} test={} shared=1 generation={generation}",
                    names.group, names.test,
                );
            }
            TAG_VERDICT => {
                let fields = read_frame(stream).map_err(|error| lost(&error))?;
                return map_verdict(names, &fields);
            }
            other => {
                return Err(names.at_stage("owner", &format!("unexpected message tag {other}")));
            }
        }
    }
}

/// The client's mapping of a verdict frame: a total match on the verdict.
fn map_verdict(names: &Names<'_>, fields: &[Vec<u8>]) -> Result<(), RunInGuestError> {
    let [verdict, generation, console] = fields else {
        return Err(names.at_stage("owner", "the verdict frame is malformed"));
    };
    let verdict = String::from_utf8_lossy(verdict);
    let generation = String::from_utf8_lossy(generation);
    let console = String::from_utf8_lossy(console);
    println!("===== shared guest {} generation {generation} =====\n{console}", names.group);
    if let Some(status) = verdict.strip_prefix("exit:") {
        return match status.parse::<i32>() {
            Ok(0) => Ok(()),
            Ok(status) => Err(RunInGuestError::BodyFailed {
                group: names.group.to_owned(),
                test: names.test.to_owned(),
                status,
            }),
            Err(_) => Err(names.at_stage("owner", &format!("unreadable exit status {status:?}"))),
        };
    }
    match verdict.as_ref() {
        "boot_failed" => Err(names.at_stage("boot", &console)),
        "guest_lost" => {
            Err(names.at_stage("guest_lost", "the shared guest was lost under this test"))
        }
        "exec_bound" => {
            Err(names
                .at_stage("exec_bound", "the test did not return within its profile's exec bound"))
        }
        other => Err(names.at_stage("owner", &format!("unknown verdict {other:?}"))),
    }
}

// ----- election ------------------------------------------------------------

fn connect(socket: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(socket)
}

/// Connects to the scope's owner, electing one first when the connect is
/// refused: take the election lock, connect once more, and otherwise start the
/// owner and wait until its socket is connectable.
fn connect_or_elect(
    endpoint: &SharedEndpoint,
    placement: &GuestPlacement,
) -> Result<UnixStream, String> {
    if let Ok(stream) = connect(&endpoint.socket) {
        return Ok(stream);
    }
    if let Some(directory) = endpoint.lock.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)
            .map_err(|error| format!("creating {}: {error}", directory.display()))?;
    }
    let lock =
        OpenOptions::new().create(true).truncate(false).write(true).open(&endpoint.lock).map_err(
            |error| format!("opening the election lock {}: {error}", endpoint.lock.display()),
        )?;
    flock(&lock, libc::LOCK_EX).map_err(|error| {
        format!("taking the election lock {}: {error}", endpoint.lock.display())
    })?;
    // An owner may have been elected while this test waited for the lock.
    if let Ok(stream) = connect(&endpoint.socket) {
        return Ok(stream);
    }
    start_owner(endpoint, placement)?;
    let started = Instant::now();
    loop {
        if let Ok(stream) = connect(&endpoint.socket) {
            // Release the lock explicitly: the owner does not hold it, and a
            // later arrival must find it free.
            let _ = flock(&lock, libc::LOCK_UN);
            return Ok(stream);
        }
        if started.elapsed() >= SCRIPTED_BOOT_BOUND {
            return Err(format!(
                "the owner did not become connectable within {SCRIPTED_BOOT_BOUND:?}; its log:\n{}",
                fs::read_to_string(endpoint.socket.with_extension("log")).unwrap_or_default()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn flock(file: &File, operation: i32) -> io::Result<()> {
    // SAFETY: `flock(2)` on a descriptor `file` owns for the call.
    let status = unsafe { libc::flock(file.as_raw_fd(), operation) };
    if status == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Starts the scope's owner: a new session and its own PID namespace, whose
/// init the owner is, so the kernel ends the guest's VMM with the owner. The
/// log of the owner, whose output is detached from this test, is the
/// socket's sibling `.log`.
fn start_owner(endpoint: &SharedEndpoint, placement: &GuestPlacement) -> Result<(), String> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(endpoint.socket.with_extension("log"))
        .map_err(|error| format!("opening the owner log: {error}"))?;
    let log_for_errors =
        log.try_clone().map_err(|error| format!("cloning the owner log: {error}"))?;
    let mut command = Command::new(&endpoint.owner_binary);
    command
        .arg(&endpoint.socket)
        .arg(placement.group)
        .arg(placement.profile())
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_for_errors);
    // SAFETY: the closure calls only async-signal-safe functions (`setsid`,
    // `unshare`, `fork`, `_exit`) and allocates nothing. It runs in the forked
    // child before `exec`: the child leaves this test's session and process
    // group (so a group signal to the test does not reach the owner), enters a
    // new PID namespace for its own children, and forks; the intermediate
    // process exits at once and the forked child, the first process of the new
    // namespace, goes on to `exec` the owner.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::unshare(libc::CLONE_NEWPID) != 0 {
                return Err(io::Error::last_os_error());
            }
            match libc::fork() {
                -1 => Err(io::Error::last_os_error()),
                0 => Ok(()),
                _ => libc::_exit(0),
            }
        });
    }
    let mut intermediate = command
        .spawn()
        .map_err(|error| format!("starting {}: {error}", endpoint.owner_binary.display()))?;
    intermediate.wait().map_err(|error| format!("reaping the owner's launcher: {error}"))?;
    Ok(())
}
