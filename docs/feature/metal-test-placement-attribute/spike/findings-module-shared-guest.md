# Probe findings — a guest shared across a module's tests

**Verdict: WORKS** for a guest shared across separate nextest test processes
(shape A), with two gaps DESIGN must close: a crashed guest owner orphans its
VMM until the run ends, and tests in a shared guest are no longer isolated from
each other.

Gate recommendation: **PROMOTE** shape A, with the guest owner, its lifetime,
the rendezvous location and the isolation and retry semantics pinned in DESIGN.

Probe sources, scripts and raw evidence:
`spike-scratch/metal-test-placement-module-guest/increment-a/` (commit
`2d32a12cc`). The `increment-a/` directory directly under `spike-scratch/`
already belonged to another feature's probe, so this probe follows the
`metal-run-isolation/increment-*` layout.

## Assumption tested

A guest booted once for a module's tests can serve several separate nextest
test processes such that: (1) each test reports its own result; (2) a failing
test is reported alone; (3) `-E 'test(=<one>)'` runs only that test; (4) one
guest boots per module per run and is gone afterwards, including after an
interrupt; (5) nothing leaks to other runs on the shared box.

## Substrate

- Host kernel `7.0.0-38-generic`, read with a plain `ssh <box> uname -r`
  because the ordinary `metal run` grammar has no command for it
  (`evidence/host-uname-r.txt`). Guests report the same kernel.
- cargo-nextest 0.9.149.
- Ordinary `cargo xtask metal run --` throughout; no `--exclusive`. The repo's
  wrapper was unchanged. The probe crate has its own `.config/nextest.toml`
  carrying the same wrapper block, because nextest reads config from the probe
  manifest's workspace root; its tests are placed through the existing
  `overdrive_test_placement` ELF marker.

## Shape A as probed

The first test of a module to arrive starts a broker (file lock in a
rendezvous directory). The broker boots one guest and serves runs over a
pathname unix socket; each test's host half connects, the broker execs the
test binary in the guest with `--exact`, and returns the exit status. The
broker exits when the anchoring nextest process is gone.

## Results

### P1, P2 — per-test results; a failure stays with its test: held

`evidence/shared-two-modules.txt`, run `a8551045337679ba58d66014`:

```text
        PASS [   2.438s] (1/6) modguest-probe::probe shared::t_pass_3
        PASS [   2.656s] (2/6) modguest-probe::probe shared_b::u_pass_1
        PASS [   2.751s] (3/6) modguest-probe::probe shared::t_pass_2
        PASS [   2.974s] (4/6) modguest-probe::probe shared_b::u_pass_2
        PASS [   3.062s] (5/6) modguest-probe::probe shared::t_pass_1
        FAIL [   3.381s] (6/6) modguest-probe::probe shared::t_fail
     Summary [   3.382s] 6 tests run: 5 passed, 1 failed, 10 skipped
    MODGUEST_BODY test=probe::shared::t_pass_3 boot_id=ea9e90f2-8e1a-4bc5-bba2-794c2af6d879 kernel=7.0.0-38-generic pid=83 uptime=0.68 0.42
    MODGUEST_BODY test=probe::shared::t_pass_2 boot_id=ea9e90f2-8e1a-4bc5-bba2-794c2af6d879 ... pid=85
    MODGUEST_BODY test=probe::shared::t_pass_1 boot_id=ea9e90f2-8e1a-4bc5-bba2-794c2af6d879 ... pid=87
    MODGUEST_BODY test=probe::shared::t_fail   boot_id=ea9e90f2-8e1a-4bc5-bba2-794c2af6d879 ... pid=89
    MODGUEST_BODY deliberate failure in probe::shared::t_fail   (exit=101)
    shared_b tests: boot_id=86438501-6b3d-4eab-8b02-69acec34b787 (second module, second boot)
```

Four tests of `shared` share one boot id with distinct pids; `shared_b` has
its own. Tests run after `t_fail` in the same guest passed.

### P3 — filtering to one test: held

`evidence/single-test-exact.txt`:

```text
        PASS [   2.260s] (1/1) modguest-probe::probe shared::t_pass_2
    MODGUEST_HOST test=t_pass_2 exit=0 runs_served=1 broker_pid=1215642 ...
     Summary [   2.260s] 1 test run: 1 passed, 15 skipped
```

### P4 — one boot per module, gone after the run: held for the run lifetime; NOT for a mid-run owner crash

Broker log, run `b8eea5fd741349aa0a5487d7` (read on the box before cleanup):

```text
MODGUEST_TIMING boots=1 staging_ms=763 boot_ms=1000 total_ms=1764
MODGUEST_BROKER ran test=shared::t_pass_3 exit=0 exec_ms=348  (then t_pass_1, t_fail exit=101, t_pass_2)
MODGUEST_BROKER exiting reason=anchor nextest process gone runs=4 lived_ms=3260
```

Interrupts (`evidence/interrupt-*.watch.txt`, first sample about 264 ms after
the signal):

```text
21:08:46.221 before: 1554930:modguest-broker 1555028:cloud-hyperviso
21:08:47.254 sending SIGINT to cargo-nextest pid 1545956
21:08:47.518 +250ms:                                    <- empty
21:10:08.520 before: 1588502:modguest-broker 1588643:cloud-hyperviso
21:10:09.679 sending SIGKILL to cargo-nextest pid 1579138
21:10:09.942 +250ms:                                    <- empty
21:11:30.895 before: 1621531:modguest-broker 1621649:cloud-hyperviso
21:11:31.946 sending SIGKILL to modguest-broker pid 1621531
21:11:32.210 +250ms:                                    <- empty (run ended: client failed on EOF)
```

The interrupted runs' broker logs have no `exiting` line: the run's own
teardown removed the broker, which was blocked in a 45 s in-guest exec.

Mid-run owner crash, with a second module keeping the run alive
(`evidence/interrupt-sigkill-broker-midrun.watch.txt`):

```text
before kill: 2 modguest-broker (1707081, 1707195) + 2 cloud-hypervisor (1707677, 1707822)
killing -9 pid 1707081
3s after kill: 1707195 broker + both cloud-hypervisor still present
8s after kill: 1707195 broker + both cloud-hypervisor still present   <- orphaned VMM
        FAIL [  20.704s] shared::t_slow      PASS [  50.002s] shared_b::u_slow
```

The killed broker's VMM survived until run teardown.

### P5 — nothing leaks to other runs: held, except the probe's own `/tmp` directory

`evidence/leaks-after-*.txt` (one clean run, four interrupted or killed runs):
no process, no `/srv/vm/metal/<run>`, no `/run/ovd-gk/<run>`, no per-run
cgroup, no `vhost-vsock` holder. Another workspace's run was live during
several of these runs with no interference. The probe left its rendezvous
directory `/tmp/modguest-<run>/` (root-owned log, lock and a stale socket),
removed by hand afterwards.

An early `ps … | grep` through ssh returned nothing while the broker and VMM
were running; those readings were discarded. Process claims come from
`procs.sh` (a `/proc/*/comm` scan) after a positive control listed the broker
and VMM mid-run.

## Timings (four tests per module: 3 pass, 1 fail)

| Shape | Boots | nextest wall | Notes |
|---|---|---|---|
| Per-test guest, serial (`-j 1`) | 4 | 11.757 s | each test: staging ~0.41 s, boot ~1.85 s, total ~2.9 s |
| Per-test guest, parallel | 4 | 5.552 s | boot 2.2-3.2 s under contention |
| A, serial | 1 | 3.626 s | first test 2.356 s including boot, then 0.42 s each |
| A, parallel | 1 | 3.382 s | broker staging 763 ms, boot 1000 ms |
| A, two modules, six tests | 2 | 3.382 s | one broker per module |
| B control: one test runs the module in one guest | 1 | 2.157 s | one nextest result for the whole module |

About 0.3 s of A's 0.42 s per later test is a 300 ms sleep the probe adds for
console flush, so the per-attach cost is an upper bound. Control B loses
per-test reporting (three passes folded into one FAIL), `-E` filtering of an
inner test and per-test retries, and needs the inner tests hidden from nextest
(the probe used `#[ignore]` plus `--include-ignored`).

## Per-test vsock namespaces

Not a conflict for shape A. Each test runs in its own local-mode vsock
namespace (the `netns=` lines differ per test); the guest is registered in the
first test's namespace, which is also the broker's. A pathname unix socket is
a filesystem object and connects across network namespaces; an abstract
socket would not. A direct vsock connect from a second namespace was not
tried; that part rests on the per-namespace CID rule in `testing.md`.

## Design implications

1. No wrapper change or new placement kind is needed: the existing marker
   suffices. The work is in the guest-kit library.
2. The shared guest needs one owner with a defined lifetime. A crashed owner
   orphans its VMM until the run ends, and an interrupt is cleaned up only by
   run teardown. DESIGN must say what makes the VMM die with its owner (for
   example a parent-death signal on the VMM, or a per-module cgroup the owner
   can kill).
3. The rendezvous must live in the run's scratch directory and go with it;
   `/tmp` leaked. The run's `TMPDIR` path exceeds the 108-byte `sun_path`
   limit, so the socket needs a short alias (for example an fd-relative
   `/proc/self/fd/<n>/…` path).
4. Sharing changes isolation: tests share one kernel, cgroup tree, nft tables
   and `/tmp`, and a nextest retry reuses the same guest. DESIGN decides
   whether sharing is opt-in per module and what a retry gets. A test that
   crashes the guest kernel was not tried.
5. One in-flight test per guest: `GuestPeer::exec` takes `&mut self`, so a
   module's parallel tests queue. Decide whether to serialise or allow
   concurrent exec.
6. nextest 0.9.149 supports setup scripts; not tried. By inspection a setup
   script would not know the test binary path, has no teardown hook, and would
   need the same socket and reaper as the lazy broker.
