# E06 — a `[job]` + `[vm]` spec deploys and its VM allocation reaches Running through the production `VmDriver` path

**Surface:** E (end-to-end) · **KPI:** K4 · **Status:** `satisfied`

<!-- Status rationale — CURRENT CAPTURE (2026-08-19, SHA fff9fe16, SEED=1,
executed on the bare-metal KVM box — NOT Lima — runner_exit_code: 0).
`satisfied` was rendered by a DIFFERENT-FOX adversarial audit of the captured
evidence (2026-08-19), not self-stamped by the runner's author — see the end
of this block. Every sub-claim passes:

  [PASS] sub-claim 0: the box is KVM-capable with cloud-hypervisor and staged artifacts
  [PASS] sub-claim 1a: deploy exited 0
  [PASS] sub-claim 2: the VM allocation reached Running within the 90s ceiling
  [PASS] sub-claim 3: new_hypervisors=1 / new_run_dirs=1 / new_scopes=1
  [PASS] no-leak: leaked_hypervisors=0 / leaked_scopes=0 / leaked_run_dirs=0 / leaked_xdp=0
  INNER_DONE serve_status=ready deploy_rc=0 final_state=Running

Three captures now exist; the arc matters because it is the K4 story:

  1. SHA 655ac964 — REFUTED. `final_state=Failed`, `no vm driver composed on
     this node`, `new_hypervisors=0`. The shipped binary had no operator-
     reachable way to compose a `Vm` driver at all. (Preserved verbatim below
     under "Prior capture (SHA 655ac964) — REFUTED".)
  2. SHA 6b6ffb12 — GREEN, but PRE-confinement. Step 03-07 (DWD-25 / ADR-0083
     §§D3a-D3c) made VM composition unconditional (gated on `Vmm::probe`) and
     moved artifacts per-allocation into the spec's own `[vm]` block, so the
     runner reached Running unmodified. At that revision this file argued
     "runner.sh was NOT modified — load-bearing"; that argument no longer
     holds and is corrected below.
  3. SHA fff9fe16 — GREEN, POST-confinement (this capture). Between capture 2
     and HEAD the confinement work landed (ADR-0082 fourth amendment): each
     launch FICLONE-clones the operator rootfs into `<data_dir>/vm/clone-
     staging` and runs cloud-hypervisor uid-dropped to OVERDRIVE_VMM_UID=4200.
     That imposes two create-time preconditions on the operator's data-dir
     which a real appliance satisfies by construction ("the appliance's one VM
     data partition") but which E06's EPHEMERAL serve data-dir did not, on the
     shared metal box:
       (a) same filesystem as the rootfs master — FICLONE is intra-filesystem;
           a cross-device master fails CLOSED as ConfinementUnavailable{UidDrop}
           on EXDEV (no copy fallback, by security control C-1). The runner had
           staged the rootfs on /srv and serve's --data-dir on /tmp.
       (b) traversable by the dropped uid — node-setup grants 0710 only on
           clone-staging, never its ancestors; `mktemp -d` makes the data-dir
           0700 (root-only), which blocked uid 4200 and made CH exit 1 before
           the guest reported ready.

`runner.sh` WAS modified for capture 3 (commit fff9fe16): serve's --data-dir is
co-located under the rootfs master's partition and granted 0711 traverse — it
models the appliance's single VM data partition. This is NOT the "implementation
diverged to make the test green" hazard the prior revision warned against, for
two independently-checkable reasons:

  - The PRODUCT is unchanged and healthy. The in-tree Tier-3 witness
    `job_plus_vm_spec_is_accepted_and_its_allocation_reaches_running` PASSES at
    HEAD on this same box — the confined-boot path boots a real guest. The
    runner change is pure substrate-modelling of the two new preconditions, not
    a product accommodation.
  - The BLACK-BOX spec surface an operator drives is identical: the same
    `[job]`+`[vm]` `render.toml`, the same `serve`/`deploy`/`describe` argv, the
    same DEFAULT-features binary (`cargo build -p overdrive-cli --bin overdrive`,
    no `--features`). Only where the runner puts serve's own data-dir changed —
    an appliance provisioning detail, not part of the operator-observed surface.

Both preconditions (a) and (b) are TEMPORARY: overdrive-fs (GH #97) supersedes
the same-filesystem requirement. No new issue is invented here.

`working_tree_dirty: true` in the manifest is benign: `dirty-status.txt` lists
only files under this expectation's own `evidence/` — the in-flight capture
recording its own output. No `crates/**` and no `runner.sh` delta is present
(runner.sh is committed at the pinned SHA fff9fe16); `dirty-diff.patch` shows
the delta is the evidence writes alone.

Different-fox audit (2026-08-19): a separate agent read ONLY the `evidence/`
files and this expectation's claim — never `runner.sh`, never `crates/**` —
prompted to REFUTE and to default to refuted on any narration, dodged
sub-claim, or number that did not add up. It independently confirmed
sub-claims 0/1/2/3 and no-leak against the pinned files and returned SATISFIED.
That is the different-fox verdict `.claude/rules/verification.md` § Enforcement
requires; it is the licence for the `satisfied` word above.

The prior REFUTED capture's rationale, cause and citations are preserved
verbatim below under "Prior capture (SHA 655ac964) — REFUTED". -->

## Expectation

An operator who has written `render.toml` declaring both `[job]` and `[vm]`,
and who runs

```
overdrive serve                       # the shipped binary, default features
overdrive deploy render.toml
overdrive workload describe e06-vm-job
```

sees the workload **accepted and scheduled**, and its VM allocation **reach
`Running` through the production `VmDriver` path** — a real Cloud Hypervisor
guest booted and dialled the vsock beacon.

This is the black-box, operator-observable form of **S-VM-39**, landed by
roadmap step `03-04` in commit `4243e849`. The in-tree Tier-3 witness is
`job_plus_vm_spec_is_accepted_and_its_allocation_reaches_running`
(`crates/overdrive-cli/tests/integration/vm_boot_failure_vocabulary.rs`), which
pins the Running row to `TransitionReason::Started` — a marker written only on
the beacon-win arm of `VmDriver::start`'s three-way boot race, so it asserts a
real guest booted. That test drives production `run_server` wiring through an
in-process composition helper. **This expectation asks the strictly harder
question no in-process helper can ask: can an operator reach the same place
through the shipped binary's own argv, with no test-only wiring?** That
question is KPI **K4**, and the feature-delta assigns its measurement to this
catalogue by name.

Step 03-07 (DWD-25 / ADR-0083 §§D3a–D3c) narrowed the gap between the two
considerably. The helper that used to carry node-level boot artifacts
(`run_with_dataplane_and_vm_artifacts`) is **deleted**: every in-tree VM test
now supplies its kernel and rootfs the way an operator does, through the
deployed spec's own `[vm]` block. The remaining difference between the Tier-3
witness and this expectation is the process boundary and the feature set — not
the artifact-supply path. The in-tree companion for the per-allocation claim
itself is **S-VM-54**
(`two_vm_jobs_on_one_serve_each_boot_from_the_rootfs_their_own_spec_named`,
`crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs`), which proves
two workloads on ONE `serve` each boot the image their own spec named.

- Anchor: S-VM-39 (`docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md:1269` — *"A VM job spec is accepted … Then the workload is accepted and scheduled / And its VM allocation reaches Running through the production VmDriver path"*)
- Anchor: roadmap 03-04 (`docs/feature/microvm-driver-cloud-hypervisor/deliver/roadmap.json:621` — criteria[0]: *"S-VM-39: a spec declaring [job] + [vm] is accepted and scheduled, and its VM allocation reaches Running through the production VmDriver path"*; `implementation_notes`: *"The test must drive real serve composition and the production VmDriver path; a parser-only acceptance assertion is insufficient"*)
- Anchor: K4 (`docs/feature/microvm-driver-cloud-hypervisor/feature-delta.md:2398` — *"The production composition path | can reach the VM driver via `overdrive serve` + `overdrive deploy` | … | A real `serve` + `deploy` boots a VM with **no** test-only wiring | Leading (the feature's pass/fail bar)"*; `:2427` assigns its measurement to *"A real `overdrive serve` + `overdrive deploy` in the verification catalogue — `verification/harness/run-expectation.sh` — Per slice — DELIVER / DEVOPS"*)
- Anchor: DWD-24 (`docs/feature/microvm-driver-cloud-hypervisor/distill/wave-decisions.md:2158` — *"S-VM-39/40 now prove the VM runs"*)
- Anchor: ADR-0083 (`docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md` — *"the registry **is** the VM capability gate"*; § D2 composes `Vm` by discovering the hypervisor binary, and a node without it *"boots normally; `[vm]` deploys are rejected at admission naming the absent capability"*)
- Anchor: ADR-0082 (`docs/product/architecture/adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md` — the `Vmm` port trait and `VmConfig` anti-corruption value the `VmDriver` path is built on)

## Verification

### Execution substrate — the bare-metal KVM box, NOT Lima

E06 declares `native-metal` in its checked-in `execution-substrate` file.
S-VM-39 boots a real Cloud Hypervisor guest, which needs x86_64 + nested KVM;
Lima on Apple Silicon provides neither.
Per `.claude/rules/testing.md` § *"Running tests — bare-metal KVM box
(`kvm-tests`)"*, the canonical transport for that surface is `cargo xtask metal
run --` against the host named by `OVERDRIVE_METAL_TARGET`. The runner uses
exactly that and never falls back to Lima — a Lima capture would be a different
claim wearing this expectation's name.

The already-reviewed pinned `evidence/verification.yaml` predates declared
substrate support and retains its historical `executed_in_lima: true` value.
Its sibling `evidence/execution_substrate.txt` is the accurate substrate record
for that capture. Fresh harness runs use the checked-in declaration and record
`execution_substrate: native-metal` plus `executed_in_lima: false`.

The box for this capture: `x86_64`, `Linux 7.0.0-29-generic`, `cgroup2fs`,
`/dev/kvm` present, `cloud-hypervisor v53.0`. Its address lives in a gitignored
`.env` and is redacted out of every captured file.

### What the runner does

Black-box throughout: the surface is the **built** `overdrive` binary's CLI
plus what the kernel exposes (`/proc`, cgroupfs, `/run/overdrive/vm`, `ip`,
`bpftool`). No `overdrive-*` crate is imported or linked.

The binary is built with **default features** — no `integration-tests`, no
`kvm-tests`. That is the measurement K4 asks for; building with the feature
flag would measure the test harness instead of the shipped composition.

**KEK delivery, in two phases, both captured.** Production `serve` composes
`SystemdCredsKeyring` and refuses to start when nothing delivers the
workload-identity KEK. Phase A starts `serve` with none and records that
fail-closed refusal (`serve_no_kek.log`) — it is why phase B has to deliver
one, so the evidence shows it rather than leaving a credential to appear out of
nowhere. Phase B then satisfies the **production** delivery contract:
`SystemdCredsKeyring::new()` reads `$CREDENTIALS_DIRECTORY/<kek_id>`, which is
precisely the file systemd materialises for a unit carrying
`LoadCredentialEncrypted=overdrive-ca-root:<path>`
(`crates/overdrive-host/src/ca/keyring.rs:22-30`, `:260-285`). The test-only
`with_credentials_dir` pin is **not** used, and neither is the gated dev
fallback `OVERDRIVE_CA_KEK` / `OVERDRIVE_CA_KEK_DEV_OPT_IN` — that would put
the capture in an explicitly non-production posture.

The guest command is `/sbin/busybox sleep 3600`, staged into a **private
reflink clone** of the shared fixture rootfs (the master at
`/srv/vm/overdrive-testing/rootfs.ext4` is read and never mutated — other
Tier-3 scenarios reuse it concurrently). The clone is needed because that
rootfs carries only `overdrive-init`, two device nodes and empty mountpoints:
a guest command that cannot exec would make `Running` a window the poller can
miss, turning a real regression into an intermittent one. The host's
statically-linked `/usr/bin/busybox` needs no cross-compiler.

Sub-claims:

0. **The box can boot a guest at all** — x86_64, `/dev/kvm`, a
   `cloud-hypervisor` binary, cgroup v2, and both staged artifacts present. A
   failure below is then the platform's, never the box's.
1. `overdrive deploy render.toml` exits `0` and prints `Accepted.` — the
   workload is admitted, and the `[service]` + `[vm]` refusal S-VM-38 proves
   does not reach the `[job]` family.
2. `overdrive workload describe <id>` shows the allocation reaching `Running`
   within a 90s ceiling — comfortably above the 30s guest-boot deadline, so a
   timeout means the allocation did not get there rather than that the poll was
   impatient. Every observation is kept (`observed_states.txt`), so the states
   the allocation passed *through* are evidence too.
3. The **production `VmDriver` path ran**, not merely that the parser accepted
   the spec: a live `cloud-hypervisor` process, a `/run/overdrive/vm/<alloc>`
   run directory and an `alloc-*.scope` cgroup that did **not** exist before
   the run. This is the black-box inverse of the in-tree test's
   `assert_no_allocation_scoped_vm_residue` fact set, stated against the same
   resources so "ran" and "left nothing behind" cannot drift apart.

`satisfied` requires all four, on a real metal run, with SHA + seed pinned in
`evidence/verification.yaml`, **and** a different-fox adversarial audit.

### The state predicate is structural, not a substring match

Worth knowing before auditing sub-claim 2, because two earlier revisions of
this runner got it wrong in the same direction — toward a **false pass**:

1. `grep -w Running` matched the empty-state diagnostic *"no allocation has
   converged to a **Running** instance yet"* — on a render showing **zero**
   allocation rows. The runner reported `Running` one second after deploy.
2. Tightening that to "a leading integer, then a word" matched the same
   diagnostic's own opening, *"**0 allocations** for workload …"*.

The predicate now anchors on the table header `workload_describe` emits
(`Attempt  State  Exit  Started  Duration`) and reads the State cell of the
first row beneath it — nothing before that header is a row, by construction.
It returns `Failed` on the captured render and `none` on the empty-state one.

The first false pass was caught because **sub-claim 3 read all zeros while
sub-claim 2 claimed Running** — a state that claims a guest is running while no
hypervisor process, run directory or cgroup scope exists. The runner now fails
the whole run on that contradiction rather than reporting 2-of-3, since
whichever probe is lying, the evidence cannot support the claim either way.

### Hygiene on a shared box

The metal box is shared with other work and with the serialized `kvm-tests`
lane, so teardown is **delta-based, never a blanket sweep**. The runner
snapshots the hypervisor processes, allocation cgroup scopes, VM run
directories, XDP attachments and network links that exist *before* it starts,
and on every exit path (an `EXIT` trap spanning staging → serve → deploy →
stop) removes only what appeared *during* the run. A blanket `pkill
cloud-hypervisor` or an all-interfaces XDP detach would kill a concurrent
guest, so neither is used. The private rootfs clone, its loop device, its
mount, the KEK credential and the keyring entry the provider parks on its miss
path are all unwound by the same trap. The workload is stopped through the
production `job stop` verb first — production spawns the VMM with
`kill_on_drop(false)`, so an orphan would contaminate the next serialized
test's `/proc` scan (the lesson S-VM-05 learned on this box).

`evidence/leak_verdict.txt` is the machine-readable proof: four counters
compared against the before-snapshot, all of which read `0`. The host-side gate
fails the run on any non-zero counter regardless of the sub-claim outcome.

**What those four counters do and do not cover.** They cover the residue that
breaks other work — hypervisor processes, allocation cgroup scopes, VM run
directories, XDP attachments. They do **not** cover the `ovd-veth-cli` /
`ovd-veth-bk` pair, and after this capture that pair is still present on the
box. That is deliberate, not an oversight: production `overdrive serve`
provisions it at boot as node infrastructure and adopts it idempotently on the
next boot (ADR-0061 converge-on-boot, whose whole point is that it "completes a
half-provisioned pair, recreates a corrupted one, never tears down a usable
one"). Tearing it down would fight that design and could disrupt a concurrent
`serve`. It carries no XDP after teardown — the post-teardown scan finds no
attachment on any interface — so it is the ordinary steady state any `overdrive
serve` leaves on a node, not this run's litter.

One observation worth recording: an `alloc-vm-exit0-0` scope and run directory
left by an **earlier, unrelated** run were present before the first E06
capture and gone before this one. `teardown_sweep.txt` shows this runner reaped
nothing, so it was not removed by E06 — consistent with production boot-time VM
reclamation (`VmHostState` is composed unconditionally per ADR-0083 § D7), but
not asserted here, since no evidence in this capture pins the mechanism.

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh E06` — SHA
`fff9fe16` (the runner-fix commit), `SEED=1`, `runner_exit_code: 0`, executed
on the metal KVM box (see `evidence/execution_substrate.txt`). `runner.sh` was
modified for this capture (co-locate serve's `--data-dir` on the rootfs
master's VM data partition + a `0711` traverse grant, so the post-`6b6ffb12`
confinement preconditions are met — see the rationale block at the top); the
change is committed at the pinned SHA, and the black-box operator surface is
unchanged.

### Per-sub-claim verdict — CURRENT capture (SHA `fff9fe16`)

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 0 | the box can boot a guest | pass | `probe_before_capability.txt`: `x86_64`, `/dev/kvm present`, `cloud-hypervisor`, `cgroup2fs`, kernel + rootfs staged. |
| 1 | deploy exits `0` and prints `Accepted.` | pass | `deploy_vm_job.meta`: `# exit: 0`; `deploy_vm_job.out` line 1 is literally `Accepted.` |
| 2 | the allocation reaches `Running` | **pass** | `INNER_DONE serve_status=ready deploy_rc=0 final_state=Running` — reached within the 90s ceiling. Previously refuted. |
| 3 | the production `VmDriver` path ran | **pass** | `resource_delta.txt`: `new_hypervisors=1`, `new_run_dirs=1`, `new_scopes=1` — a real Cloud Hypervisor guest was spawned. Previously `0/0/0`. |
| — | no leak | pass | `leaked_hypervisors=0`, `leaked_scopes=0`, `leaked_run_dirs=0`, `leaked_xdp=0`. |

The binary under test is built with DEFAULT features (`binary_under_test.txt`
/ `build.log`: `cargo build -p overdrive-cli --bin overdrive`, no
`--features`), so no `integration-tests` or `kvm-tests` wiring participates.

The verdict word in the header is `satisfied`: a different-fox adversarial
audit (2026-08-19) read only the captured `evidence/` — never `runner.sh`,
never `crates/**` — prompted to refute, and independently confirmed all four
sub-claims plus no-leak. Per `.claude/rules/verification.md` § Enforcement, the
runner's author does not self-stamp; the audit is the entity that rendered the
verdict, and this capture is the evidence it read.

---

## Prior capture (SHA `655ac964`) — REFUTED

Preserved verbatim. This is the capture that measured K4 as NOT MET and
motivated DWD-25 / step 03-07.

Captured under `evidence/` by `harness/run-expectation.sh E06` — SHA
`655ac964`, `SEED=1`, `runner_exit_code: 1`, executed on the metal KVM box
(see `evidence/execution_substrate.txt`; `working_tree_dirty: true` records
this expectation's own untracked files, listed in `dirty-status.txt`).

| File | What it shows |
|---|---|
| `execution_substrate.txt` | historical metal-not-Lima transport record for the pinned pre-declaration capture |
| `metal_preflight.out` | the box answered ssh; arch/kernel/user (host redacted) |
| `probe_before_capability.txt` | `x86_64` · `Linux 7.0.0-29-generic` · `/dev/kvm present` · `cloud-hypervisor v53.0` · `cgroup2fs` · both staged artifacts |
| `probe_before_{hypervisors,scopes,run_dirs,xdp}.txt` | the pre-run state teardown must restore — all empty |
| `serve_no_kek.log` | phase A: `KEK unavailable at boot; control-plane refusing to start (no throwaway KEK minted)` |
| `kek_provisioning.txt` | phase B: the production systemd-creds delivery contract, and what was deliberately not used |
| `rootfs_staging.log` | the private reflink clone and the statically-linked busybox staged into it; master untouched |
| `spec_render.toml` | the exact `[job]` + `[vm]` spec deployed |
| `binary_under_test.txt` / `build.log` | `cargo build -p overdrive-cli --bin overdrive` with no `--features` |
| `serve.log` | the production `overdrive serve` that bound and served this capture |
| `deploy_vm_job.out` / `.meta` | sub-claim 1: the verbatim accept render and `# exit: 0` |
| `describe_final.out` / `describe_poll_trail.out` / `observed_states.txt` | sub-claim 2: the final render, every poll, and the state trail |
| `resource_delta.txt` | sub-claim 3: counts of allocation-scoped VM resources that appeared |
| `stop_vm_job.out` / `describe_after_stop.out` | the production stop verb driving the workload down |
| `teardown_sweep.txt` / `leak_verdict.txt` / `probe_post_teardown_*.txt` | what the sweep reaped, and the four-counter no-leak proof |

### Per-sub-claim verdict — PRIOR capture (SHA `655ac964`)

| # | Sub-claim | Verdict | Reason |
|---|---|---|---|
| 0 | the box can boot a guest | pass | `probe_before_capability.txt`: `x86_64`, `/dev/kvm present`, `cloud-hypervisor v53.0`, `cgroup2fs`, kernel + rootfs staged. The box is not the limiting factor. |
| 1 | deploy exits `0` and prints `Accepted.` | pass | `deploy_vm_job.meta`: `# exit: 0`; `deploy_vm_job.out` line 1 is literally `Accepted.`, followed by the full `workload_submit_accepted` render. The S-VM-38 `[service]` refusal did not reach the `[job]` family. |
| 2 | the allocation reaches `Running` | **refuted** | The allocation went to `Failed` and stayed there for the full 90s ceiling — 45 consecutive polls, `observed_states.txt`. It never passed through `Running`. |
| 3 | the production `VmDriver` path ran | **refuted** | `resource_delta.txt`: `new_hypervisors=0`, `new_run_dirs=0`, `new_scopes=0`. No hypervisor was spawned, so nothing allocation-scoped was ever created. |

The operator surface names the cause itself — `describe_final.out`, verbatim:

```
Job 'e06-vm-job' (kind: Job)
Spec digest: 0825be61fcea077d74c34cb1538e79cc3c248db48a05825a2ef9c50fdb4e43f4
Verdict: Failed (backoff exhausted)

Attempt  State        Exit   Started              Duration
1        Failed       —      (c=6,w=local)        —

    reason: driver internal error: no vm driver composed on this node
    error: no vm driver composed on this node
```

### Why sub-claims 2 and 3 are refuted — and what that does and does not mean

**It is not a regression in step 03-04, and it does not impeach the in-tree
S-VM-39 test.** The refusal happens strictly upstream of anything that step
landed: the shipped binary has **no way to be told where the kernel and rootfs
artifacts live**, so it never composes a `Vm` driver at all, and the
dispatch-time fallback then reports exactly the text above.

The chain, cited rather than narrated:

- `crates/overdrive-control-plane/src/lib.rs:1593-1594` — the whole
  discover → probe → insert block that composes the `Vm` driver is
  `#[cfg(feature = "integration-tests")]` **and** guarded by
  `if let Some(artifacts) = config.vm_artifacts.clone()`.
- `crates/overdrive-control-plane/src/lib.rs:940-957` — `ServerConfig::
  vm_artifacts` is itself `#[cfg(feature = "integration-tests")]`. Its own doc
  comment states the intent plainly: *"`None` (production default): the
  composition root never attempts to compose a `Vm` driver at all — the
  identical capability-absence posture as a host with no `cloud-hypervisor`
  installed"*, and *"no ADR pins how the PRODUCTION composition root's
  kernel/rootfs paths are supplied — that is DEVOPS's
  `infra/metal/provision.sh` territory, out of this step's scope."*
- `crates/overdrive-cli/src/main.rs:201` — the `Command::Serve` arm calls
  `commands::serve::run(args)`, the production path, which leaves
  `vm_artifacts` unset. The three siblings that set it
  (`run_with_dataplane_and_vm_artifacts`, `run_with_vm_artifacts`,
  `run_with_dataplane_and_vmm_override` —
  `crates/overdrive-cli/src/commands/serve.rs:168-222`) are each
  `#[cfg(feature = "integration-tests")]` and reachable only as Rust
  functions. No argv, env var or config file reaches them.
- `crates/overdrive-control-plane/src/action_shim/mod.rs:1358-1365` — with no
  `Vm` entry in the registry, `StartAllocation` synthesises
  `DriverStartFailure { class: Unclassified { driver: Vm }, detail: "no Vm
  driver composed on this node" }` and the allocation lands `Failed`. That is
  the string `describe_final.out` carries.

So the shape is: **the mechanism composes, and no production path reaches it.**
The feature's own risk register named this in advance —
`feature-delta.md:2445`, *"The mechanism composes but no production path
reaches it (precedent warning #1 — the reference implementation's exact
failure) … K4 is a named, binary KPI"* — and CLAUDE.md § *"Build vertical
slices through production entry points"* describes the same failure mode. E06
is the instrument that measures it, and today it reads **K4 not yet met**.

Two things this expectation deliberately does **not** claim:

1. **It does not claim a defect.** `ServerConfig::vm_artifacts`'s own doc
   comment scopes the production artifact-supply path out of the slice that
   landed it and names DEVOPS as its owner. This is a documented boundary; E06
   records where that boundary currently sits, executed rather than asserted.
2. **It does not claim S-VM-39 is unproven.** The in-tree Tier-3 test genuinely
   boots a guest and reaches `Running` on this same box, and it is the
   `what, forever` witness for the scenario. E06 is the `why` for the
   operator-facing half, and that half is what is outstanding.

E06 becomes capturable end-to-end the moment the composition root gains an
operator-reachable way to supply the kernel and rootfs artifacts — at which
point this runner re-runs unchanged and sub-claims 2 and 3 become live. No
GitHub issue is cited, because none has been approved for creation (CLAUDE.md
§ "Deferrals require GitHub issues" forbids an agent creating one unilaterally
and forbids inventing a number).
