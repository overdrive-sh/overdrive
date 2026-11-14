//! Shared helpers of the Metal scenarios: drive a fixture test of this binary
//! as a child process (as nextest would) and read what the guest kit and the
//! shared-guest owner left behind. The scenarios read only what the design
//! pins: the `OVERDRIVE_GUEST_KIT_TIMING` lines, process exit and output, and
//! the owner binary's process.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// The Metal run variables a child inherits unless a scenario removes them.
pub const METAL_VARIABLES: [&str; 3] =
    ["OVERDRIVE_METAL_RUN_ID", "OVERDRIVE_METAL_EXECUTION_CLASS", "OVERDRIVE_METAL_SOURCE_ROOT"];

/// Names the in-guest scenario body a scenario client hands to the guest.
pub const CLIENT_BODY_ENV: &str = "OVERDRIVE_PLACEMENT_SCENARIO_BODY";
/// Makes the scenario body fail inside the guest.
pub const FAIL_ENV: &str = "OVERDRIVE_PLACEMENT_SCENARIO_FAIL";
/// Makes the scenario body hold for this many seconds inside the guest.
pub const HOLD_SECONDS_ENV: &str = "OVERDRIVE_PLACEMENT_SCENARIO_HOLD_SECONDS";
/// A host path a guest-placed fixture writes when its body runs in place.
pub const MARKER_ENV: &str = "OVERDRIVE_PLACEMENT_SCENARIO_MARKER";

pub const OWNER_BINARY: &str = "overdrive-shared-guest-owner";

/// One `OVERDRIVE_GUEST_KIT_TIMING` line.
#[derive(Debug, Clone)]
pub struct Timing {
    fields: BTreeMap<String, String>,
}

impl Timing {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    pub fn shared(&self) -> bool {
        self.field("shared") == Some("1")
    }

    pub fn generation(&self) -> Option<u32> {
        self.field("generation")?.parse().ok()
    }
}

pub fn timings(output: &str) -> Vec<Timing> {
    output
        .lines()
        .filter_map(|line| line.split_once("OVERDRIVE_GUEST_KIT_TIMING ").map(|(_, rest)| rest))
        .map(|rest| Timing {
            fields: rest
                .split_whitespace()
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        })
        .collect()
}

pub fn shared_generations(output: &str) -> BTreeSet<u32> {
    timings(output).iter().filter(|t| t.shared()).filter_map(Timing::generation).collect()
}

pub struct Completed {
    pub success: bool,
    pub output: String,
}

fn command(test: &str) -> Command {
    let exe = std::env::current_exe().expect("this test binary's path");
    let mut command = Command::new(exe);
    command.args(["--exact", test, "--nocapture"]);
    command
}

/// Runs `test` of this binary to completion, the way nextest runs one test, and
/// returns its combined output.
pub fn run_test(test: &str, environment: &[(&str, &str)], remove: &[&str]) -> Completed {
    let mut command = command(test);
    for (name, value) in environment {
        command.env(name, value);
    }
    for name in remove {
        command.env_remove(name);
    }
    let output = command.output().expect("spawning the fixture test");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Completed { success: output.status.success(), output: text }
}

/// A fixture test running in the background; its output lines arrive on a
/// channel so a scenario can wait for the guest kit's attach line.
pub struct Running {
    pub child: Child,
    lines: Receiver<String>,
    seen: String,
}

pub fn spawn_test(test: &str, environment: &[(&str, &str)], own_process_group: bool) -> Running {
    let mut command = command(test);
    for (name, value) in environment {
        command.env(name, value);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if own_process_group {
        command.process_group(0);
    }
    let mut child = command.spawn().expect("spawning the fixture test");
    let (sender, lines) = mpsc::channel();
    for stream in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let sender = sender.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
    }
    Running { child, lines, seen: String::new() }
}

impl Running {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Waits until a line containing `needle` has been printed, or `bound`
    /// elapses.
    pub fn wait_for_line(&mut self, needle: &str, bound: Duration) -> bool {
        if self.seen.contains(needle) {
            return true;
        }
        let deadline = Instant::now() + bound;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.seen.push_str(&line);
                    self.seen.push('\n');
                    if line.contains(needle) {
                        return true;
                    }
                }
                Err(_) => return false,
            }
        }
        false
    }

    /// Kills the process, then returns what it printed.
    pub fn finish_after_kill(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(500)) {
            self.seen.push_str(&line);
            self.seen.push('\n');
        }
        self.seen.clone()
    }

    /// Waits for the process to end and returns everything it printed.
    pub fn finish(mut self) -> Completed {
        let status = self.child.wait().expect("waiting for the fixture test");
        while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(500)) {
            self.seen.push_str(&line);
            self.seen.push('\n');
        }
        Completed { success: status.success(), output: self.seen }
    }
}

// ----- processes of this Metal run -----------------------------------------

fn processes() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .expect("reading /proc")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .collect()
}

/// Host pids of the shared-guest owner processes in this run's cgroup tree.
/// The scenario runs in its own cgroup namespace rooted at the run's cgroup,
/// so a process outside the run reads as a path that climbs out (`/..`).
pub fn owners_in_this_run() -> BTreeSet<u32> {
    processes()
        .into_iter()
        .filter(|pid| {
            std::fs::read_link(format!("/proc/{pid}/exe"))
                .ok()
                .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().into_owned()))
                .is_some_and(|name| name == OWNER_BINARY)
        })
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
                .is_ok_and(|text| text.lines().all(|line| !line.contains("::/..")))
        })
        .collect()
}

/// Samples [`owners_in_this_run`] every 100 ms from a background thread until
/// stopped, so a scenario sees every owner that served it even if one exits
/// idle or is killed before the scenario looks. The scenarios assume the gap
/// between the clients of one scenario is far below the owner's 30 s idle
/// period (`SHARED_GUEST_IDLE`, design DDD-17), so one owner serves a scope
/// unless the scenario itself kills it.
pub struct OwnerSampler {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    seen: std::thread::JoinHandle<BTreeSet<u32>>,
}

impl OwnerSampler {
    pub fn start() -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&stop);
        let seen = std::thread::spawn(move || {
            let mut seen = BTreeSet::new();
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                seen.extend(owners_in_this_run());
                std::thread::sleep(Duration::from_millis(100));
            }
            seen.extend(owners_in_this_run());
            seen
        });
        Self { stop, seen }
    }

    /// Stops sampling and returns every owner pid seen while it ran.
    pub fn stop(self) -> BTreeSet<u32> {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.seen.join().expect("the owner sampler thread")
    }
}

fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = stat.rsplit_once(')')?.1;
    after_name.split_whitespace().nth(1)?.parse().ok()
}

/// Every process descended from `root` by parentage as the host sees it.
pub fn descendants_of(root: u32) -> BTreeSet<u32> {
    let parents: Vec<(u32, u32)> =
        processes().into_iter().filter_map(|pid| Some((pid, parent_of(pid)?))).collect();
    let mut found = BTreeSet::new();
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for (pid, ppid) in &parents {
            if *ppid == parent && found.insert(*pid) {
                frontier.push(*pid);
            }
        }
    }
    found
}

pub fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
        && std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            !stat.rsplit_once(')').is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
        })
}

pub fn kill(pid: i32, signal: i32) {
    // SAFETY: `kill(2)` with a pid the scenario spawned or found by name.
    let status = unsafe { libc::kill(pid, signal) };
    assert_eq!(status, 0, "kill({pid}, {signal}) failed");
}

pub fn wait_until(bound: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn test_path(module: &str, name: &str) -> String {
    format!("integration::{module}::{name}")
}

#[allow(dead_code, reason = "a scenario may name the binary path in a failure message")]
pub fn this_binary() -> PathBuf {
    std::env::current_exe().expect("this test binary's path")
}
