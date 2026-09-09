# E09-v2 VM allocation stop: premise validation

Date: 2026-09-08. Investigator: Codex, repository nw-troubleshooter role.
Source: `b653e1ad1758d11be33be457849b284f78141333`.
Status: BLOCKED on the existing Sim composition boundary for the actual
public-stop timeout. The original cleanup-ordering premise is unsupported;
no seeded product-invariant failure reproduced. No production change made.

## Scope and method

Investigate the supported public `job stop` VM allocation path and its transient
cgroup, run-directory, rootfs-clone, and clone-index cleanup. Canonical workload
intent and Service/dataplane deletion are outside this investigation.

Loaded role skills: `nw-investigation-techniques` and
`nw-five-whys-methodology`. Read AGENTS.md, its eight mandatory project files,
and `.context/02-04-vm-stop-cleanup-bug-handoff.md`. Preserve all existing dirty
work and native evidence. No DES events, commits, native operations, or new
production/Sim API are part of this investigation.

Initial source/evidence reading was exploration. It found that the handoff's
interpretation of `healthy-resources-active` conflicts with the runner's
capture point. The following diagnostic probes are recorded before execution.

## Probe P1 — native capture chronology

- Hypothesis: the cited resource lists were captured before stop, and the
  capture does not establish that they remained after terminal publication.
- Predicted outcome: `healthy-resources-active` is captured before the first
  `stop_workload`; cases 4–9 have no `healthy-resources-after` capture; the
  final resource counts are unchanged across shutdown, with no case 4–9
  allocation in the final snapshot.
- Falsification: a cited list is populated after terminal publication, a
  post-terminal residue capture names these allocations, or the final snapshot
  shows their artifacts disappearing only across shutdown.

## Probe P2 — existing seeded production-owner controls

- Hypothesis: the existing real `VmDriver` orderly-stop owner and registered
  VM Artifact Disposal control still terminate/dispose their allocations on
  current source.
- Predicted outcome: the two focused seeded controls pass and print their
  seeds; no code changes are needed to make them pass.
- Falsification: either control fails on VMM termination, resource disposal,
  or preservation of the existing terminal ending.
- Limit: these are positive controls, not a regression for the disputed
  public-stop cleanup-ordering claim. A passing control cannot exclude a
  different native branch.

## Probe P3 — compare successful and timed-out stop populations

- Hypothesis: the native stop failures are consistent with a serialized stop
  queue exceeding the runner's 60-second terminal wait, rather than missing
  disposal after terminal publication.
- Predicted outcome: successful initial stops and failed workers' cleanup
  retries show approximately 12-second spacing; retry transcripts say
  `AlreadyStopped`; failed cases lack the post-stop artifact sample because
  they exited before reaching that sample. Production has a single serial
  convergence drain, with each VM stop allowed two seconds plus ten seconds
  before cleanup.
- Falsification: the timings are unrelated to the serial stop budget, or a
  failed worker has a post-terminal residue sample persisting beyond the
  cleanup observation bound.
- Limit: timing agreement is evidence for a candidate mechanism, not the
  required seeded production-loop invariant. Do not turn it into a scheduling
  change or accepted DESIGN requirement on this evidence alone.

## Probe P4 — public VM cohort composition and accepted time contract

Continuation authorized the actual healthy-stop timeout investigation through
the serial convergence owner. Initial exploration identified the following
testability hypothesis, recorded before the bounded source-identity probe.

- Hypothesis: existing per-tick Sim network injection cannot reach the actual
  production convergence-loop owner; boot composes host network/VM-artifact
  adapters, and the accepted stop contract has no per-cohort sixty-second
  terminal deadline.
- Predicted outcome: the public server's config exposes no workload-network
  provisioner or VM-host-state substitution; the private loop calls the
  ordinary tick, which dispatches through `HostNetworkProvisioner`. VM starts
  require that provisioner even with a Sim dataplane. The checked-in sixty-second
  per-stop limit is an example observation budget; accepted design specifies
  serial dispatch and bounded per-VM waits.
- Falsification: an existing public server/owner composition path accepts these
  existing Sim ports, an actual VM deployment bypasses host provisioning, or
  an accepted contract explicitly promises this cohort terminal within sixty
  seconds. If a composition path exists, use it for the seeded trajectory;
  otherwise report the precise missing boundary without adding one.
- Limit: this is a source-boundary diagnostic, not an execution of the desired
  seeded cohort invariant. It cannot prove the timing mechanism or classify
  latency as a product defect.

## Result: the handoff overstates its native evidence

P1 confirmed its prediction. The runner captures `healthy-resources-active`
at `examples/service-kind-vm-workloads-v2/run-example.sh:893`, before stopping
the healthy peer Job at line 913 and the healthy Service at line 918.
The post-stop capture is a different file, `healthy-resources-after`, written
at line 924 only after `stop_workload` succeeds. Cases 1, 2, 3, and 10 have that
post-stop sample; cases 4–9 do not. The four paths cited in the handoff for
cases 4–8 are **pre-stop inventory**, not post-terminal residue.

The failure at line 919 exits the worker before recording the original stop
duration and before changing `WORKER_HEALTHY_STATE` from `Running`. Its EXIT
trap then calls `stop_workload` again and overwrites the same stop/describe
files (`run-example.sh:809–822`). That explains the retained ledger's `Running`
value beside the final case 4–8 `Terminated` transcripts and `already stopped`
response. These are different moments. The final transcript cannot establish
when the first stop's wait failed or when resources disappeared.

The final capture says, verbatim (`evidence/run.log:1370–1374`):

```text
final_runtime_resources_before_shutdown:
vm=0	cgroup=204
final_runtime_resources_after_shutdown:
vm=0	cgroup=204
```

No `unexpected new ... after owned cleanup` diagnostic appears. The runner
executes its baseline comparison before shutdown at lines 1226–1231 and again
afterward at lines 1235–1240. Its final snapshot contains none of the case 4–9
allocation identifiers. This supports cleanup before shutdown, and contradicts
the handoff's claim that the cited artifacts disappeared *only* during shutdown.
The 204 scopes are a host-wide count, not 204 scopes owned by this run.

**Do not use the current handoff as proof of a terminal-before-cleanup defect.**
The expectation still failed; rejecting this interpretation does not turn E09
green or justify relaxing its assertions.

## Production entry point, ownership, ordering, and retry

| Boundary | Current source evidence |
|---|---|
| CLI public verb | `crates/overdrive-cli/src/main.rs:139` dispatches `JobCommand::Stop`; `commands/deploy.rs:399–406` calls `ApiClient::stop_workload`; `http_client.rs:282–283` sends `POST /v1/workloads/{id}/stop`. |
| HTTP route and handler | `crates/overdrive-control-plane/src/lib.rs:3135` registers `handlers::stop_workload`; `handlers.rs:835–889` checks canonical intent, atomically writes the separate stop sentinel, removes listener facts, and enqueues workload-lifecycle. It returns acceptance of intent, not completion of allocation cleanup. |
| Serialized runtime owner | `lib.rs:3348–3398` drains the broker and awaits each `run_convergence_tick` in one `for` loop, then sleeps or observes shutdown. Each drained evaluation has the same tick number. There is no per-evaluation timeout cancelling a stop at `TickContext.deadline`. |
| Reconciler decision | `crates/overdrive-reconcilers/src/workload_lifecycle.rs:549–570` emits operator `StopAllocation` only for `Running` rows while the declared workload's stop sentinel exists. Hydration filters rows by owning workload at lines 445–454. |
| Driver routing and awaited stop | `crates/overdrive-control-plane/src/action_shim/mod.rs:800–810` resolves the allocation-to-driver index, falling back to composed drivers on a miss; lines 2812–2832 await `driver.stop`, tolerating only `DriverError::NotFound`. |
| VM ending transition | `crates/overdrive-worker/src/vm_driver.rs:1651–1701` extracts a matching `Live` entry and atomically replaces it with `EndingInFlight`; no live entry yields `NotFound`. Lines 1715–1757 await the two-second beacon deadline, VMM termination with ten-second grace, and host cleanup before returning. Some cleanup errors are discarded, which is a source fact, not a reproduced cause here. |
| Terminal publication | `action_shim/mod.rs:2837–2953` finishes mTLS/network teardown, re-reads the current row, proposes the operator terminal ending through `write_alloc_lifecycle`, and releases supervision. The allocation-to-driver routing index is removed at line 2962. Terminal writes have a fixed two-proposal contention bound; read/write failures release the claim and return a typed error. |
| Retry | `reconciler_runtime.rs:1517–1608` retains the dispatch outcome, self-enqueues when actions were emitted, and then returns the error. A later tick rehydrates actual state; `Running` can produce another stop, terminal rows do not. |
| Natural-exit owner | `vm_driver.rs:2108–2169` drains the guest/VMM ending, waits for Running release, atomically changes the originating live session to `EndingInFlight`, and sends an exit event; it performs no host cleanup. `worker/exit_observer.rs:284–301` releases supervision after every Wrote/Failed/NoWrite outcome. |
| Existing artifact disposal | `vm_reclamation.rs:155–194` excludes supervised allocations and selects disposal for unclaimed VM-exclusive artifacts, including the no-desired-entry branch. Lines 245–291 declare a 30-second node sweep. Hydration's VM intent join currently enumerates Jobs (lines 360–372), but a Service's run directory/clone still qualifies through the VM-exclusive no-entry branch. `action_shim/reclamation.rs:287–305` claims, kills, and discards; its lease Drop releases at lines 76–80. |
| Shutdown/cancellation | `lib.rs:1406–1422` cancels and joins the convergence owner before later shutdown stages; the live tick is drained. The loop checks cancellation between batches. The writer's bounded abort at `vm_driver.rs:1722` is inside the awaited stop protocol, followed by VMM termination and cleanup. A fabricated abort of the stop future is not evidence of this production path. |

## Three required hypotheses

| Hypothesis | Disposition |
|---|---|
| H1: `VmDriver::stop` ran, a cleanup operation failed, and its error was discarded | The discard sites exist at `vm_driver.rs:1741,1752–1755`. No captured syscall error or post-terminal residue demonstrates that this branch caused E09's failure. Remains unproven; no fault was injected to manufacture it. |
| H2: `NotFound` after ending/exit claim race left no artifact-disposal owner | The branch is present, but the actual exit observer releases its claim and the VM-exclusive artifact planner permits disposal after release. No native trace or seeded execution demonstrates the proposed permanent owner gap. Remains unproven. |
| H3: the existing VM reclaimer failed to converge leftovers | P2's registered disposal control passed. Native final comparison reports no new runtime resources before shutdown. No observed continuing residue proves this failure. Remains unproven for the E09 incident. |

None is promoted to a fix or DESIGN requirement.

## Candidate explanation for the actual stop timeouts

P3 confirmed the timing prediction. The runner prints case timing files in
lexical path order (`run-example.sh:1167–1170`), so the sequence is 1, 10, 2,
3, 4, 5, 6, 7, 8, 9. Initial healthy Service stops that returned successfully
were case 10: 12,594 ms; case 1: 24,922 ms; case 2: 37,239 ms; case 3:
49,600 ms. The failed workers' EXIT-trap retries recorded 2,497, 14,850,
26,102, 38,475, 50,874, and 59,757 ms for cases 4–9. The original failed
call's duration is not retained; it exits before `record_timing`.

The approximately twelve-second staircase matches the serial production
owner and `VM_SHUTDOWN_REQUEST_DEADLINE = 2s`, `VM_STOP_GRACE = 10s`
(`vm_driver.rs:68–73`). `CloudHypervisorVmm::terminate` actually waits for a
process exit or grace expiry (`crates/overdrive-host/src/vmm.rs:534–568`).
The runner allows only sixty seconds to observe a terminal allocation
(`run-example.sh:695–709`), separately from the later cleanup wait.

Five causal levels, with their evidence limits made explicit:

1. E09 fails at `healthy-stop` for cases 4–9: native ledger and failure lines.
2. The first public-stop helper did not finish within its observation budget:
   runner's failure site; the later cleanup retry overwrote its transcript.
3. Stops execute serially in a shared drained evaluation batch: production
   `spawn_convergence_loop` caller path.
4. Each still-live VMM stop may consume roughly twelve seconds: two existing
   deadlines, with native successful-stop and retry timing staircases.
5. Ten concurrent workloads can therefore wait longer than sixty seconds
   before their stop action completes. This is a source-supported **candidate
   liveness mechanism**, not a reproduced cleanup violation or an accepted
   scheduling defect. No causal claim about why the guest uses the complete
   grace window is made without guest/VMM evidence.

## Executed seeded controls

The first P2 invocation selected `overdrive-sim/integration-tests` only and
failed compilation because the existing tick wrapper is gated by
`overdrive-control-plane/integration-tests`. This was an invocation issue;
it provided no behavioral result and required no source edit. Corrected,
bounded command:

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim \
  --features integration-tests,overdrive-control-plane/integration-tests \
  --test vm_finalize_failed_ownership_spike --test e10_vm_early_exit_spike \
  -E 'test(vm_orderly_stop_retains_owner_until_termination_control) | test(no_intervening_restart_reclamation_preserves_authored_ending_control)' \
  --no-capture
```

Run ID: `ecdc9158-2b8f-4690-b50f-ac4cc00288ff`. Exit 0. Relevant output:

```text
e10-vm-early-exit seed=257205 restart=false
seed=257205: registered reclamation control passed; terminal and occurrences unchanged
vm-finalize-ownership seed=257204 finalize=false
owned_before_stop=true live_before_stop=true subsequent_stop=Ok(()) live_after_stop=false
Summary [0.231s] 2 tests run: 2 passed, 2 skipped
```

The two skipped tests were excluded by the explicit test filter. Neither
passing control is represented as a failing public-stop regression.

## Continued investigation: contract and production-owner testability

P4 confirmed the missing composition boundary. Read-only source probe output:

```text
P4 source HEAD: b653e1ad1758d11be33be457849b284f78141333
ServerConfig adapter-related fields: clock, dataplane_override, mtls_identity_override, vmm_override
ServerConfig workload_network_provisioner present: False
ServerConfig VmHostState present: False
production convergence owner private: True
loop uses ordinary run_convergence_tick: True
loop uses injected tick wrapper: False
crates/examples diff against native source: (empty)
```

The actual route is reachable in the product: the native runner invokes public
`job stop`; the handler's accepted sentinel enqueues the registered reconciler;
the boot-owned loop serially awaits its actions; the awaited VM stop drains
before terminal publication. The source table above identifies every boundary.
The obstacle is reproducing that **same whole owner trajectory** in Sim with
the existing injectable host ports:

| Existing boundary | Why it cannot drive this requested Sim cohort |
|---|---|
| Public `run_server_with_obs_and_drivers`, `lib.rs:2115` | Accepts the registry needed for a real `VmDriver` backed by Sim VMM/cgroup adapters, but `ServerConfig` (`lib.rs:731–992`) does not carry `WorkloadNetworkProvisioner` or `VmHostState`. |
| Actual convergence owner, `lib.rs:3066–3071,3318–3398` | Private function spawned by boot with its own `AppState`. It calls ordinary `run_convergence_tick`, not the existing per-tick provisioner wrapper. There is no exposed owner entry accepting the manually constructed Sim state/provisioner combination. |
| Ordinary tick, `reconciler_runtime.rs:1352–1360` | Calls inner tick with provisioner `None`; production dispatch chooses `HostNetworkProvisioner` (`action_shim/mod.rs:863–883`). |
| VM deployment, `action_shim/mod.rs:1180–1217` | Requires host network assignment even when mTLS is absent under a Sim dataplane. Provisioning performs real netns/veth/TAP effects (`mod.rs:766–780`) before `Driver::start`. A Sim dataplane override does not substitute this port. |
| Existing per-tick wrapper, `reconciler_runtime.rs:1372–1390` | Supports the registered reconciler and action-shim path with an injected provisioner, but does not run the production loop's drain, ordering, cadence, or shutdown ownership. A test-written serial loop would choose the very ordering under investigation. |
| VM artifact observation, `lib.rs:2769–2795` | Public boot constructs `RealVmHostState` with host cgroup and `/run/overdrive/vm` roots unconditionally, so it would not observe the simulated driver's host-state adapter. The public `AppState.vm_host_state` field can be replaced in manually composed per-tick fixtures, but that is not the boot-owned state of the public server. |

This is a specific **testability blocker**, not a proposed production defect.
It is not solved by starting real host network operations in a seeded Sim test,
manually inserting Running/terminal rows, substituting an Exec workload for the
VM cohort, or recreating the owner loop in the test. None of those alternatives
was executed. No missing signature was invented.

There is also a timing-model limitation in the existing controls:
`SimVmm::terminate` takes `_grace` and completes immediately
(`crates/overdrive-sim/src/adapters/vmm.rs:410–441`). Its documented equivalence
is the killed outcome, not elapsed grace (`vmm.rs:8–20`). Thus P2 cannot verify
or falsify a twelve-second queue trajectory. A timing-faithful implementation
of the existing VMM port would still leave the full-owner network/host-state
composition blocker above; changing the Sim adapter was not necessary to
establish that blocker and was not done.

### Accepted stop contract versus the runner's observation budget

The following sources do not establish a sixty-second public cohort promise:

- ADR-0023, `adr-0023-action-shim-placement.md:253–283`, explicitly accepts
  inline sequential dispatch. The 100 ms cadence is the broker drain rate,
  not a timeout that cancels a dispatched stop.
- ADR-0082, `adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md:1444–1502`,
  pins the two-second request deadline, ten-second process grace, cleanup,
  and operator terminal outcome. It specifies per-driver work, not the sum
  of queueing delays for ten workloads. Its consequences at lines 2185–2189
  already acknowledge cumulative serial dispatch stalls for VM starts; this
  investigation does not reopen that control-plane-wide deferral.
- S-SVM-25, `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md:57–80`,
  requires twenty pairs at concurrency ten, truthful guest outcomes, no
  discarded/replaced/retried trials, and reclamation. Its 600-second setup and
  trials budget plus sixty-second final cleanup grace explicitly is a
  functional sample, not a native throughput guarantee.
- The independent **per-stop terminal observation** cutoff is introduced by
  `run-example.sh:695–711`. Its sixty seconds precede a separate cleanup wait;
  it is not the S-SVM-25 final remote-owner cleanup grace. The handler itself
  reports whether the stop sentinel was inserted, including `AlreadyStopped`
  while convergence is outstanding (`handlers.rs:853–860,882–891`).

Consequently the twelve-second staircase is consistent with a runner
observation-budget mismatch against accepted serial behavior. That is the
strongest bounded classification supported here, **not a proven product
liveness defect** and not a definitive diagnosis of the initial helper phase.
The overwritten original transcript and absent intermediate resource inventory
still prevent distinguishing its terminal-wait timeout from its later
cleanup-wait timeout conclusively. No expectation was relaxed and E09 remains
failed.

## Bounded disposition and blocker

**There is no failing seeded public-stop regression command or seed to report.**
The existing positive-control command/output and seeds are retained above;
they are not substituted for the missing reproduction. The native seed-1
failure remains evidence of the actual E09 journey failure, not a Sim invariant.

H1/H2/H3 remain unproven. The serial-latency explanation has matching native
timings and a concrete production owner path, but the existing Sim composition
cannot exercise that full path with simulated VM network/host-state ports.
The bounded disposition is to return this precise **DESIGN/testability gap**
for an explicit decision about exposing the existing adapter boundary to the
production owner. This report specifies the missing capability, not a new API
shape or authorization to implement it. It does not recommend scheduling,
persistence, recovery, timeout, or expectation changes.

Once that boundary is explicitly resolved, the remaining diagnostic is a
seeded public VM cohort trajectory that measures acceptance, stop start/end,
terminal publication, and artifact removal under the current owner. Its oracle
must distinguish the runner's observation budget from an accepted product
contract. Until then, no cleanup repair or latency remediation is justified by
this investigation.

## Preservation and exact changes

Only this Markdown file was added. No production code, diagnostic Rust test,
design, roadmap, expectation, existing evidence, or DES log was changed; no
commit was created. Pre-existing dirty paths remain untouched. Current HEAD
matches the native capture, and `git diff --name-only` against that SHA for
`crates/` and `examples/` is empty.

Original evidence retained verbatim:

- `verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/run.log`
  SHA-256 `55ea1b8e8df9462ca731c5a6e6e062fbc53afcab4cba243b13bd17d304c5514d`.
- `verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/evidence/verification.yaml`
  SHA-256 `7c95b077dcf42674eef671950cc94991b30e9fa630d871a8c0930ccba0bdf5ee`.
