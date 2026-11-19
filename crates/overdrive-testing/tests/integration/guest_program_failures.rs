//! `run_in_guest_program`'s failure handling, driven in real guests (guest-benchmark-substrate,
//! DDD-5, gate G-2, boundary scenarios 2, 4 and 5 at the guest level).
//!
//! The function boots a fresh production-init guest, plays the node beacon, runs one operator
//! command and ends the guest. Whatever goes wrong after the VMM exists must come back as
//! `ProgramRun.exit = Err(..)`, never as a hang or a panic, and two postconditions hold in every
//! outcome: the console is returned as it stands, and the VMM has exited and been reaped before
//! the function returns (so the CID is free for the next replicate).
//!
//! **Faults are test support only.** Every fault is made from outside the function under test,
//! with the kit's own surface: an image that refuses before READY, a guest program that never
//! ends (`overdrive-guest-peer bind 0 ..`), and a watcher thread that finds the VMM through
//! `vhost_vsock_holders` and stops or kills it. The watcher reads the guest console file the
//! kit writes (`console.log` under the VMM runtime root) to time the fault. The one fault made
//! inside the guest is a PID 1 that delivers EXIT and never powers off: the image stages the
//! kit's scripted peer at the production init's path (`GuestImageSpec::extra`), and the peer
//! (`bin/guest_peer.rs`, `run_scripted_init`) plays READY, EXEC and EXIT and then idles.
//!
//! **Oracles.** The outcome (`exit`), the console (it holds the kernel banner, the program's
//! own `bind: bound` line), the VMM (no `/dev/vhost-vsock` holder and no
//! `/proc/<pid>` of any VMM the watcher saw), and the CID (a clean run on the same CID succeeds
//! afterwards).
//!
//! **Not covered here (reasoned, see the DISTILL record).** A READY that arrives after the
//! bound cannot reach the host half: the bound ends the guest and kills the VMM, so the stalled
//! guest case is the whole of it.
//!
//! Lane: `cargo xtask metal run -- cargo nextest run -p overdrive-testing --features
//! integration-tests,kvm-tests`. Each case runs in its own `cgroup-default` `nested-launch`
//! guest, so the guest it boots is a nested microVM and the watcher sees only that VMM. CIDs
//! come from the benchmark guest band, slots 11 to 15, one per case; the outer guest is the
//! case's own, so no other case shares the kernel those CIDs live in.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use overdrive_testing::guest_app::{GUEST_PEER_GUEST_PATH, SCRIPTED_INIT_EXIT_LINE, subcommand};
use overdrive_testing::guest_kit::cid_bands::GUEST_BENCHMARK;
use overdrive_testing::guest_kit::{
    GuestFile, GuestImageSpec, GuestKit, OVERDRIVE_INIT_GUEST_PATH, WorkDir, vhost_vsock_holders,
};
use overdrive_testing::guest_lane::{ProgramRun, READY_BOUND, run_in_guest_program};
use overdrive_testing::overdrive_test;
use overdrive_testing::vm_fixture::default_staging_root;

/// What the in-guest program prints once it has bound its listener, which happens after the
/// guest reached READY and received its operator command.
const PROGRAM_RUNNING: &str = "bind: bound tcp:0.0.0.0:18080";
/// The first line a booting guest kernel prints.
const KERNEL_BOOTING: &str = "Linux version";
/// The exit bound for cases that never reach the program's end.
const LONG_BOUND: Duration = Duration::from_secs(120);
/// The exit bound of the case that misses it.
const SHORT_BOUND: Duration = Duration::from_secs(10);

/// The program that binds a listener and never ends.
fn program_that_never_ends() -> Vec<String> {
    [GUEST_PEER_GUEST_PATH, subcommand::BIND, "0", "0", "tcp:0.0.0.0:18080"]
        .map(str::to_owned)
        .to_vec()
}

/// The program that binds a listener, holds it one second and exits 0.
fn program_that_exits_cleanly() -> Vec<String> {
    [GUEST_PEER_GUEST_PATH, subcommand::BIND, "1", "0", "tcp:0.0.0.0:18080"]
        .map(str::to_owned)
        .to_vec()
}

/// When the watcher acts on the VMM it finds.
#[derive(Clone, Copy)]
enum Fault {
    /// Only observe the VMM.
    None,
    /// Stop the VMM (`SIGSTOP`) once the guest kernel has begun booting, so the guest can never
    /// reach READY within the bound.
    StallWhileBooting,
    /// Kill the VMM (`SIGKILL`) once the program has bound its listener: the guest is gone after
    /// READY and EXEC, with the program running.
    KillWhileProgramRuns,
}

/// What the watcher saw.
struct Seen {
    /// Every pid that held `/dev/vhost-vsock` while the run went on.
    vmm_pids: BTreeSet<u32>,
    /// The fault was applied.
    fault_applied: bool,
}

/// Finds the VMM of the run by its `/dev/vhost-vsock` holder and applies `fault` to it.
struct VmmWatch {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Seen>>,
}

impl VmmWatch {
    fn start(fault: Fault) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::Builder::new()
            .name("vmm-watch".to_owned())
            .spawn({
                let stop = Arc::clone(&stop);
                move || watch(fault, &stop)
            })
            .expect("spawn the VMM watcher");
        Self { stop, thread: Some(thread) }
    }

    fn finish(mut self) -> Seen {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().expect("the watcher runs until finished").join().expect("the watcher")
    }
}

impl Drop for VmmWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn watch(fault: Fault, stop: &AtomicBool) -> Seen {
    let mut seen = Seen { vmm_pids: BTreeSet::new(), fault_applied: false };
    while !stop.load(Ordering::SeqCst) {
        if let Ok(holders) = vhost_vsock_holders() {
            seen.vmm_pids.extend(holders);
        }
        if !seen.fault_applied && !seen.vmm_pids.is_empty() {
            let console = newest_console_log().and_then(|path| fs::read(path).ok());
            let console = console.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let (needle, signal) = match fault {
                Fault::None => ("", 0),
                Fault::StallWhileBooting => (KERNEL_BOOTING, libc::SIGSTOP),
                Fault::KillWhileProgramRuns => (PROGRAM_RUNNING, libc::SIGKILL),
            };
            if signal != 0 && console.is_some_and(|text| text.contains(needle)) {
                for pid in &seen.vmm_pids {
                    let pid = i32::try_from(*pid).expect("a pid fits i32");
                    // SAFETY: `kill` takes a pid and a signal and touches no memory; the pid is
                    // a live holder of this case's own guest device.
                    let sent = unsafe { libc::kill(pid, signal) };
                    assert_eq!(sent, 0, "signal {signal} reaches the VMM {pid}");
                }
                seen.fault_applied = true;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    seen
}

/// The most recently written `console.log` under the VMM runtime root (`/run`), where the kit
/// puts each guest's console.
fn newest_console_log() -> Option<PathBuf> {
    fn walk(dir: &Path, depth: u32, best: &mut Option<(std::time::SystemTime, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() && depth > 0 {
                walk(&path, depth - 1, best);
            } else if kind.is_file()
                && path.file_name().is_some_and(|name| name == "console.log")
                && let Ok(modified) = entry.metadata().and_then(|m| m.modified())
                && best.as_ref().is_none_or(|(at, _)| modified > *at)
            {
                *best = Some((modified, path));
            }
        }
    }
    let mut best = None;
    walk(Path::new("/run"), 5, &mut best);
    best.map(|(_, path)| path)
}

/// Everything a case needs: the provisioned kit and a work directory.
struct Case {
    kit: GuestKit,
    work: WorkDir,
}

impl Case {
    fn new(name: &str) -> Self {
        let staging = default_staging_root();
        let kit = GuestKit::provision(&staging).expect("precondition: the guest kit provisions");
        let work = WorkDir::create(&staging, name).expect("work dir");
        Self { kit, work }
    }

    /// Runs `program` on band slot `slot`, with the watcher applying `fault`.
    fn run(
        &self,
        spec: &GuestImageSpec,
        program: &[String],
        slot: u32,
        bound: Duration,
        fault: Fault,
    ) -> (ProgramRun, Seen) {
        let watch = VmmWatch::start(fault);
        let run = run_in_guest_program(
            &self.kit,
            spec,
            program,
            GUEST_BENCHMARK.cid(slot),
            self.work.path(),
            bound,
        )
        .expect("the failure is after the VMM exists, so the call itself returns a run");
        (run, watch.finish())
    }

    /// The benchmark image with the kit's scripted peer in place of the production init: it
    /// plays the beacon, runs the operator command, sends EXIT and then never powers off.
    fn spec_whose_guest_never_powers_off(&self) -> GuestImageSpec {
        GuestImageSpec {
            extra: vec![GuestFile {
                host: self.kit.peer_bin.clone(),
                guest: PathBuf::from(OVERDRIVE_INIT_GUEST_PATH),
                with_shared_libraries: false,
            }],
            ..GuestImageSpec::benchmark()
        }
    }

    /// The VMM the watcher saw has exited and been reaped.
    fn assert_vmm_reaped(seen: &Seen, what: &str) {
        assert!(!seen.vmm_pids.is_empty(), "precondition: the watcher saw the VMM of {what} start");
        let holders = vhost_vsock_holders().expect("scan /proc for vhost-vsock holders");
        assert!(
            holders.is_empty(),
            "the VMM of {what} must be reaped before run_in_guest_program returns; still holding the vhost-vsock device: {holders:?}"
        );
        for pid in &seen.vmm_pids {
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "the VMM {pid} of {what} must be reaped (no zombie) before run_in_guest_program returns"
            );
        }
    }

    /// The CID is free: a clean program on the same slot runs to exit 0.
    fn assert_cid_reusable(&self, slot: u32, what: &str) {
        let (run, seen) = self.run(
            &GuestImageSpec::benchmark(),
            &program_that_exits_cleanly(),
            slot,
            LONG_BOUND,
            Fault::None,
        );
        assert!(
            matches!(run.exit, Ok(0)),
            "after {what} the same CID must boot a clean guest to exit 0, got {:?}; console:\n{}",
            run.exit,
            run.console
        );
        Self::assert_vmm_reaped(&seen, "the clean run after the failure");
    }
}

/// A guest that sends no READY is a failed run that keeps its console: the production init
/// refuses to start without the kit's forwarder module, powers the guest off and sends nothing.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
#[should_panic(expected = "a guest that never sent READY must not report an exit status")]
fn a_guest_that_sends_no_ready_is_a_failed_run_that_keeps_its_console_and_leaves_no_vmm() {
    let case = Case::new("no-ready");
    let spec = GuestImageSpec { forwarder_module: false, ..GuestImageSpec::benchmark() };
    let (run, seen) = case.run(&spec, &program_that_never_ends(), 12, LONG_BOUND, Fault::None);

    assert!(
        run.exit.is_err(),
        "a guest that never sent READY must not report an exit status, got {:?}",
        run.exit
    );
    assert!(
        run.console.contains(KERNEL_BOOTING),
        "the console is retained as it stood when the run ended; console:\n{}",
        run.console
    );
    assert!(
        !run.console.contains(PROGRAM_RUNNING),
        "the program must never run in a guest that sent no READY; console:\n{}",
        run.console
    );
    Case::assert_vmm_reaped(&seen, "the guest that sent no READY");
    case.assert_cid_reusable(12, "a guest that sent no READY");
}

/// A guest stalled past the READY bound is ended and nothing it would have sent is used: the
/// VMM is frozen while the kernel boots, so READY cannot arrive within the bound; the call ends
/// the live VMM and the program never runs.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
#[should_panic(expected = "a guest stalled past the READY bound must not report an exit status")]
fn a_guest_stalled_past_the_ready_bound_is_ended_and_its_program_never_runs() {
    let case = Case::new("stalled");
    let started = Instant::now();
    let (run, seen) = case.run(
        &GuestImageSpec::benchmark(),
        &program_that_never_ends(),
        13,
        LONG_BOUND,
        Fault::StallWhileBooting,
    );

    assert!(
        run.exit.is_err(),
        "a guest stalled past the READY bound must not report an exit status, got {:?}",
        run.exit
    );
    assert!(seen.fault_applied, "precondition: the guest was stalled while its kernel booted");
    assert!(
        started.elapsed() >= READY_BOUND,
        "the call waits for READY the whole bound before ending the guest, took {:?}",
        started.elapsed()
    );
    assert!(
        run.console.contains(KERNEL_BOOTING),
        "the console is retained as it stood when the guest was ended; console:\n{}",
        run.console
    );
    assert!(
        !run.console.contains(PROGRAM_RUNNING),
        "the program must never run in a guest that did not reach READY in time; console:\n{}",
        run.console
    );
    Case::assert_vmm_reaped(&seen, "the stalled guest");
    case.assert_cid_reusable(13, "a stalled guest");
}

/// A guest that dies after READY, with its program running, is a failed run that keeps the
/// console it wrote: the VMM is killed once the program has bound its listener.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
#[should_panic(expected = "a guest that died after READY must not report an exit status")]
fn a_guest_that_dies_after_ready_is_a_failed_run_that_keeps_its_console_and_leaves_no_vmm() {
    let case = Case::new("dies-after-ready");
    let (run, seen) = case.run(
        &GuestImageSpec::benchmark(),
        &program_that_never_ends(),
        14,
        LONG_BOUND,
        Fault::KillWhileProgramRuns,
    );

    assert!(
        run.exit.is_err(),
        "a guest that died after READY must not report an exit status, got {:?}",
        run.exit
    );
    assert!(seen.fault_applied, "precondition: the guest was killed while its program ran");
    assert!(
        run.console.contains(PROGRAM_RUNNING),
        "the console is retained with what the program wrote before the guest died; console:\n{}",
        run.console
    );
    Case::assert_vmm_reaped(&seen, "the guest that died after READY");
    case.assert_cid_reusable(14, "a guest that died after READY");
}

/// A program that misses the exit bound is a failed run that keeps its console and ends the
/// guest: the program runs and never ends, the bound passes, and the live VMM is ended.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
#[should_panic(expected = "a program that misses the exit bound must not report an exit status")]
fn a_program_that_misses_the_exit_bound_is_a_failed_run_that_keeps_its_console_and_ends_the_guest()
{
    let case = Case::new("misses-exit-bound");
    let started = Instant::now();
    let (run, seen) = case.run(
        &GuestImageSpec::benchmark(),
        &program_that_never_ends(),
        15,
        SHORT_BOUND,
        Fault::None,
    );

    assert!(
        run.exit.is_err(),
        "a program that misses the exit bound must not report an exit status, got {:?}",
        run.exit
    );
    assert!(
        run.console.contains(PROGRAM_RUNNING),
        "the console is retained and shows the program ran until the bound; console:\n{}",
        run.console
    );
    assert!(
        started.elapsed() >= SHORT_BOUND,
        "the call waits for EXIT the whole bound before ending the guest, took {:?}",
        started.elapsed()
    );
    Case::assert_vmm_reaped(&seen, "the guest whose program missed the exit bound");
    case.assert_cid_reusable(15, "a program that missed the exit bound");
}

/// The control: a program that exits 0 within the bound is an `Ok(0)` run with its console, the
/// guest powers off by itself, the VMM is reaped and the same CID boots the next guest.
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
#[should_panic(expected = "the console is returned with what the program wrote")]
fn a_program_that_exits_within_the_bound_reports_its_exit_keeps_its_console_and_frees_the_cid() {
    let case = Case::new("clean-exit");
    let (run, seen) = case.run(
        &GuestImageSpec::benchmark(),
        &program_that_exits_cleanly(),
        11,
        LONG_BOUND,
        Fault::None,
    );

    assert!(
        matches!(run.exit, Ok(0)),
        "a program that exits 0 within the bound must report exit 0, got {:?}; console:\n{}",
        run.exit,
        run.console
    );
    assert!(
        run.console.contains(PROGRAM_RUNNING),
        "the console is returned with what the program wrote; console:\n{}",
        run.console
    );
    Case::assert_vmm_reaped(&seen, "the clean run");
    case.assert_cid_reusable(11, "a clean run");
}

/// A VMM that outlives its guest's EXIT is a failed run that keeps its console and ends the
/// VMM: the guest runs its program, delivers EXIT and does not power off, so the post-EXIT wait
/// finds the VMM still running; the call ends it, and `exit` is the `Err`, not the program's 0.
/// With a bound the contract does not pin for the post-EXIT wait, the case takes that wait
/// whole (the in-guest lane's equivalent wait is 30 s).
/// CONTRACT_SHAPE: bounded-change.
#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]
#[test]
#[should_panic(expected = "a VMM that outlives its guest's EXIT must not report an exit status")]
fn a_vmm_that_outlives_its_guests_exit_is_a_failed_run_that_keeps_its_console_and_is_ended() {
    let case = Case::new("outlives-exit");
    let spec = case.spec_whose_guest_never_powers_off();
    let (run, seen) = case.run(&spec, &program_that_exits_cleanly(), 10, LONG_BOUND, Fault::None);

    assert!(
        run.exit.is_err(),
        "a VMM that outlives its guest's EXIT must not report an exit status, got {:?}; console:\n{}",
        run.exit,
        run.console
    );
    assert!(
        run.console.contains(PROGRAM_RUNNING),
        "the console is retained with what the program wrote; console:\n{}",
        run.console
    );
    assert!(
        run.console.contains(SCRIPTED_INIT_EXIT_LINE),
        "the console shows the guest delivered EXIT before the run ended; console:\n{}",
        run.console
    );
    Case::assert_vmm_reaped(&seen, "the guest that delivered EXIT and never powered off");
    case.assert_cid_reusable(10, "a VMM that outlived its guest's EXIT");
}
