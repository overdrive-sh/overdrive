<!-- markdownlint-disable MD024 -->
# Feature delta — `metal-run-isolation`

DISCUSS, 2026-11-07. Requirements ready for DESIGN; implementation is not authorised by this document. Developer tooling with a developer-visible verification outcome. Density: `lean`, `ask-intelligent`, `explicit_override` from the installed density resolver. One job, one story, one release slice. No product-runtime behaviour changes.

## Wave: DISCUSS / [REF] Persona ID

`marcus-platform-verification-engineer` — Marcus, the Overdrive engineer running verification from multiple Git worktrees against the shared Metal box; a coding agent may invoke the same commands on his behalf. [Persona](../../product/personas/marcus-platform-verification-engineer.yaml).

## Wave: DISCUSS / [REF] JTBD one-liner

`J-PLAT-006`: When independent Overdrive changes need verification on the same Metal box, I want each worktree's ordinary verification to proceed concurrently in disposable guests with its own source and evidence, so I can decide whether to continue or revise each change without waiting for an unrelated test execution lease or trusting contaminated results. [Job register](../../product/jobs.yaml).

## Wave: DISCUSS / [REF] Premise and evidence

This is a bounded new tooling capability, grounded in the existing runner, not a claim of a newly reproduced product defect. The user selected Git-worktree identity with branch fallback, the existing fresh disposable guest per isolation group, and nWave fast-forward. DISCOVER and DIVERGE were unnecessary for this explicit request; their absence is not customer-research evidence. The prior exploration is recorded in `.context/metal-run-isolation-handoff.json`; maintained source citations below make the handoff independent of that local file.

| Observed code fact | Evidence at DISCUSS |
|---|---|
| Remote source and build directories derive from sanitised branch names; an environment override exists. Same-branch workspaces and different names with the same sanitisation can alias. | `infra/metal/bootstrap.sh:35-55` |
| The box-wide execution lease and singleton owner record coexist with a per-checkout lock and source digest. Removing only the first flock is insufficient. | `infra/metal/bootstrap.sh:132-141,279-285,308`; `infra/metal/lease-holder.sh:551-578`; `infra/metal/native-preflight.sh` |
| `metal run` forwards the supplied command; it does not automatically select the disposable-guest suite. | `xtask/src/main.rs:987-1005` |
| Durable execution already has invocation-specific records, combined logs, exact command statuses, and read-only repeatable collection. | `infra/metal/README.md:40-75`; `infra/metal/lease-holder.sh:279-307,715-723` |
| The existing guest suite boots fresh guests for its named isolation groups, including explicitly nested boundary cells. | `crates/overdrive-testing/bin/fwd_guest_suite.rs:1-26,2934-2973` |
| Test-support CID bands are static, and a reachable pre-cut beacon listener uses a fixed host vsock port. They need a concurrent-use contract before lease removal. | `crates/overdrive-testing/src/guest_kit.rs:232-299`; `crates/overdrive-testing/src/guest_lane.rs:250-253` |
| The current documented `kmod-load` step loads the physical host's module; raw host nextest can mutate host kernel state. | `infra/metal/README.md:104-118`; `.config/nextest.toml` host-kernel group |
| Authoritative direct-native Cloud Hypervisor evidence currently requires a non-virtualised x86_64 launcher and forbids substituting nested results. | `.claude/rules/testing.md:1795-1816` |

The operator entry point for this feature is the existing developer command `cargo xtask metal run -- COMMAND`, not `overdrive serve` or a manufactured product CLI. No expensive remote run or defect reproduction was performed in DISCUSS. Collision risks above are code observations, not measured failure rates.

## Wave: DISCUSS / [REF] Locked decisions

| ID | Direction | Verdict and boundary |
|---|---|---|
| D1 | Identify remote state by Git worktree, with branch fallback. | User-approved direction, recorded in the orchestrator handoff. Exact identity, fallback eligibility, overrides and migration semantics remain DESIGN-owned. |
| D2 | Preserve the existing fresh disposable guest per isolation group. | User-approved direction. No new outer guest per entire run and no general replacement framework. |
| D3 | Ordinary verification from independent worktrees must not hold or wait for one box-wide test execution lease. | Requested outcome. Keep invocation IDs and per-checkout source/artifact exclusion. |
| D4 | Physical-host provisioning and kernel maintenance remain explicitly exclusive. | Preserved requirement. The exact maintenance admission interface is not accepted here. |
| D5 | Preserve kernel isolation, native evidence applicability, durable statuses, source fidelity and cleanup ownership. | Invariants; concurrency is not permission to weaken them. |

These rows do not accept new public types, methods, CLI flags, environment variables, wire/record fields, error variants, allocation protocols or lifecycle contracts. Any such material choices must be proposed and surfaced by DESIGN before acceptance.

## Wave: DISCUSS / [REF] User stories with elevator pitches

### US-METAL-ISOLATION — Verify independent changes concurrently and trust each result

`job_id: J-PLAT-006`; priority: must-have; slice: [concurrent worktree verification](slices/slice-01-concurrent-worktree-verification.md).

Marcus works in `ouagadougou` while another change uses `san-antonio` on the same Metal box. Today an unrelated command can hold the box-wide lease even when it has no kernel effects, and branch-derived remote state does not distinguish every worktree. He needs independent verification results without moving ordinary test kernel mutations onto the physical host.

### Elevator Pitch
Before: Marcus waits behind another worktree's ordinary verification and cannot rely on branch names alone to separate remote source and artifacts.
After: run `cargo xtask metal run -- cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests` from each prepared worktree → sees each invocation's `OVERDRIVE_METAL_RUN_ID=...`, its `===== disposable guest core =====` output, and its own `OVERDRIVE_METAL_RUN_COMPLETE run_id=... exit_code=... cause=...`.
Decision enabled: Marcus can use each worktree's source-matched command result and retained log to continue or revise that change independently.

### Domain examples

1. Marcus starts the existing suite in `ouagadougou` and `san-antonio`, each prepared from its own source. Their execution intervals overlap, their logs identify different invocations, and finishing either leaves the other running.
2. In a controlled acceptance setup, two distinct worktrees check out `marcus-sa/metal-isolation` with different uncommitted source changes. They retain distinct remote source/build state; branch equality does not collapse their identities. Branches `metal/isolation` and `metal-isolation` likewise must not alias merely because sanitisation matches.
3. Marcus loses the controller connection after a run starts, then uses `cargo xtask metal collect <run-id>`. The run continues and collection returns that command's actual result, including status 255, while a different worktree keeps running.

### UAT scenarios and embedded acceptance criteria

```gherkin
Scenario: Independent worktrees verify together
  Given ouagadougou and san-antonio have prepared verification inputs and the box has capacity
  When Marcus starts the existing guest suite through metal run from both worktrees
  Then both command execution intervals overlap without waiting for the other ordinary run's box-wide lease
  And each invocation produces its own run id, guest-group output, retained log and exact completion status

Scenario: Worktree identity keeps results attributable
  Given two distinct Git worktrees use the same branch but have different source contents
  When Marcus verifies both through metal run
  Then each command uses only its worktree's synced source and matching build artifacts
  And different branch names with identical sanitised spellings cannot alias remote state

Scenario: One checkout cannot change under its running command
  Given a durable command is executing from ouagadougou's remote checkout
  When Marcus requests another run, sync or pull against that same checkout
  Then checkout exclusion preserves the executing command's source and artifact consistency
  And this exclusion does not block a different worktree's ordinary verification

Scenario: Verification changes the disposable guest kernel
  Given the existing verification selection mutates module, nftables, BPF, route or test cgroup state
  When Marcus invokes ordinary verification through metal run, including a direct test command
  Then the test's kernel effects occur only in its fresh disposable guest isolation group
  And the physical host has zero test-owned changes to those kernel surfaces
  And run-owned VMM launcher resources are distinguished from the kernel state under test

Scenario: One invocation cannot consume or clean up another's resources
  Given two worktrees execute the same guest isolation group concurrently
  When one invocation finishes or fails through an existing supported terminal path
  Then guest identities and host listeners do not collide or deliver evidence to the wrong invocation
  And its cleanup releases only its owned guests, CIDs, listeners, images, temporary files and processes
  And the surviving invocation continues with its source, artifacts and evidence intact

Scenario: A disconnected controller does not lose the verification result
  Given a durable command has reached OVERDRIVE_METAL_RUN_STARTED
  When Marcus's controller disconnects and he later collects that run id twice
  Then execution has not been cancelled by the controller loss
  And both collections return the retained log and actual command status, including 255
  And runner failures remain distinguishable from a command result

Scenario: Concurrent verification preserves qualification and maintenance boundaries
  Given the selection includes an existing explicitly nested boundary cell or requires direct-native evidence
  When Marcus verifies it on the Metal box
  Then each result is credited only for its approved execution boundary
  And missing native qualification or required guest inputs cannot produce successful evidence
  And physical-host kernel maintenance is admitted only with exclusive ownership excluding ordinary runs
```

AC-1 through AC-7 correspond in order to these scenarios. AC-2 includes worktree discovery with branch fallback under D1; DESIGN must pin precisely when that fallback is eligible. AC-3 permits concurrent requests from one worktree to serialize; simultaneous execution from one checkout is not required. AC-4 covers direct `metal run -- cargo nextest ...` paths as well as explicit guest-suite invocations: documenting a recommended wrapper alone does not meet the outcome. Pure Sim/build commands need no test-kernel exclusion; DESIGN determines routing while preserving command meaning.

AC-5 applies to existing failure/cancellation owners and terminal paths, not invented controller-cancellation or crash-recovery mechanisms. AC-6 preserves the existing honest supervisor-loss and malformed-record outcomes; it does not promise reboot-resumable runs. AC-7 does not relabel nested results as direct-native evidence or prohibit the already accepted named nested-boundary cells. Their precise applicability must be reconciled during DESIGN before execution routing can be accepted.

### System constraints

- Zero cross-worktree source/artifact/evidence mixing, zero concurrent run-resource collisions, and zero cleanup of another invocation's resources.
- Zero ordinary test-owned physical-host module, nftables, BPF, route or test cgroup mutations. Required host VMM launch resources, including launch confinement resources, must be explicitly named, owned and cleaned up; they are not the kernel subsystem under test.
- Keep one fresh disposable guest per existing isolation group. No compulsory guest around a whole invocation. Preserve exact command selection, completion status, durable registration/start boundaries and repeatable read-only collection.
- No promised speedup ratio or unlimited concurrency: acceptance assumes capacity for two selected ordinary runs. Removal of box-wide test serialization is observable through overlapping execution, not a throughput benchmark.

## Wave: DISCUSS / [REF] Story map and scope assessment

| Choose source | Prepare verification | Invoke independently | Observe execution | Decide and retain |
|---|---|---|---|---|
| Select each Git worktree | Existing build / guest-kit preparation | Existing `metal run` in both worktrees | Separate run IDs, guest logs and completion statuses | Collect each run; continue or revise its change |

Scope assessment: **PASS** — one developer-tooling context, one user-visible outcome, one story with seven scenarios. The change spans existing shell runner, Rust tooling and guest fixtures; this is integration within one outcome, not three new product bounded contexts. One slice covers the entire backbone. Identity-only or lease-removal-only technical releases cannot satisfy this outcome safely.

Priority: the single slice validates the central concurrency-and-isolation hypothesis first. Estimate: one working day / six crafter hours as a planning hypothesis, to be checked by DESIGN after the bounded contract questions below. This is not an effort waiver or permission to omit invariants. No new abstractions or independent future slices are assumed.

## Wave: DISCUSS / [REF] Outcome KPIs

Objective: Marcus can assess two independent worktree changes concurrently on the shared Metal box while trusting each result.

| KPI | Who does what | Target | Baseline | Measurement and timeframe |
|---|---|---|---|---|
| K1, primary | Marcus completes paired ordinary verification from distinct worktrees without unrelated execution-lease waiting | 2/2 runs overlap; 0 box-wide ordinary execution-lease waits | Existing runner admits at most one command under its box-wide lease | One paired dogfood capture at delivery using actual worktree contents, start/completion logs and process intervals; not a benchmark or measured success-rate claim |
| K2, guardrail | Marcus attributes each result to its selected source and can collect it after controller loss | 100% of the contrasting captured runs retain the right source/log/status; 2/2 repeat collections agree | Durable result/collection contract exists; concurrent worktree identity not yet provided | Existing durable records, source digest and collected output at delivery; include one nonzero result |
| K3, guardrail | Marcus runs ordinary kernel-mutating verification without contaminating the physical test host or the other run | 0 forbidden physical-host test-kernel deltas; 0 surviving ephemeral guest/launcher resources from the completed invocation; 0 cleanup-attributable changes to the other invocation's resources | Guest-group isolation exists; raw host mutation path still documented | Bounded before/after kernel/resource ownership observations for the same paired delivery capture; exact surfaces and attribution pinned in DESIGN |

Owner: feature implementer captures, independent reviewer evaluates. Existing runner logs, checkout source markers and external resource observations are the data sources; no analytics service, persistent dashboard, recurring schedule or new benchmark is required. Missing whole-suite timing data is not a prerequisite to removing the observed execution lease.

## Wave: DISCUSS / [REF] Definition of Done

- [ ] AC-1: distinct-worktree ordinary command execution overlaps, including same-branch identity coverage.
- [ ] AC-2: worktree/source/artifact/evidence attribution is exact, including sanitisation-collision and approved fallback cases.
- [ ] AC-3: same-checkout run/sync/pull exclusion remains effective across controller loss.
- [ ] AC-4: ordinary test kernel mutations stay in the fresh guest isolation groups; host launch-resource boundary is explicit.
- [ ] AC-5: concurrent invocation resources and cleanup remain disjoint through supported terminal paths.
- [ ] AC-6: durable logs, actual statuses and collection/failure distinctions are preserved.
- [ ] AC-7: direct-native and named nested-boundary evidence retain their approved meanings; maintenance remains exclusive.
- [ ] DESIGN's exact contracts and semantic error mappings are approved; DISTILL coverage and independent implementation reviews satisfy repository gates.
- [ ] The one slice is dogfooded through the real developer entry points with retained source/substrate evidence and matching operator documentation.

## Wave: DISCUSS / [REF] Definition of Ready

| Item | Status | Evidence |
|---|---|---|
| Clear domain problem | PASS | Existing unrelated-run serialization and ambiguous remote identity, with source citations |
| Specific persona | PASS | Marcus, multi-worktree Metal verification engineer; persona SSOT |
| Three concrete examples | PASS | Existing workspace pair, same-branch/colliding-spelling setup, controller loss |
| Three to seven UAT scenarios | PASS | Seven scenarios through existing developer commands |
| Derived testable ACs | PASS | AC-1..AC-7; zero-delta invariants and observable overlap/status outputs |
| Right-sized | PASS, DESIGN estimate check | One vertical slice; six-hour planning hypothesis; no scope truncation permitted |
| Technical constraints | PASS | D1..D5 and system constraints distinguish test kernels, launchers and maintenance |
| Dependencies resolved or tracked | PASS for DESIGN | Existing runner/suite reused; exact DESIGN-owned questions recorded below |
| Numeric outcome KPIs | PASS | K1..K3 with baseline, target, owner and bounded measurement |

Requirements completeness: **1.00 (9/9 applicable DoR checks)**. This is a requirements check, not approval of design feasibility, test implementation or delivery. No unresolved user-outcome question blocks starting DESIGN. Material interface contracts below must be resolved, validated and approved before DISTILL/DELIVER. Per-wave peer review is skipped under the DISCUSS skill's optional review rule; consolidated downstream review remains required.

## Wave: DISCUSS / [REF] Out-of-scope

Resuming or changing `service-allocation-readiness-and-ending`; changing Overdrive product runtime semantics; simultaneous source-mutating operations inside one checkout; replacing the guest suite with a general scheduler/framework; a guest around every whole run; remote-host fleets; indefinite capacity guarantees; durable-run reboot recovery or pruning; new dashboards or benchmarks; speculative resource-recovery subsystems; new product CLI/API surface. No deferrals or new GitHub issues are created.

## Wave: DISCUSS / [REF] WS strategy

**A — real existing infrastructure**. Brownfield developer command → existing Metal runner → existing fresh guest groups → actual retained output. The single slice is the minimum end-to-end flow; no stubbed transport, new public port or environment-switching strategy is proposed.

## Wave: DISCUSS / [REF] Driving ports

Existing `cargo xtask metal run -- COMMAND`, `metal sync`, `metal pull <path>...`, `metal collect <run-id>` and existing guest-suite executable/group selection. Existing `metal shell` and bootstrap/provisioning are physical-host administrative boundaries requiring a precise DESIGN applicability decision. No new flags, subcommands or environment names are accepted. The elevator pitch uses the already documented explicit suite invocation; routing of other ordinary commands is still a design contract.

## Wave: DISCUSS / [REF] Pre-requisites and DESIGN handoff

The existing configured Metal target, qualified native x86_64/KVM launcher, kernel/rootfs inputs, guest-kit preparation and sufficient capacity for two selected runs remain prerequisites. DISCUSS changes no substrate. Source/job/persona/journey SSOTs link this developer outcome without borrowing a product-operator job.

DESIGN must read the actual code, not turn this requirements handoff into asserted mechanisms. Cover all of these contract questions together:

| Question | Required DESIGN result before downstream execution |
|---|---|
| Q1 — worktree identity | Exact stable identity and branch-fallback eligibility; same-branch and sanitisation collisions; explicit remote-directory override behaviour; sync/run/pull/source-marker consistency; bounded existing-state migration. |
| Q2 — runner ownership | Remove ordinary box-wide test serialization and singleton-owner assumptions while preserving checkout exclusion, durable start/result boundaries and exclusive physical-host maintenance. Pin owner, lifetime, failure/status representation and affected compatibility contracts. |
| Q3 — host-shared guest resources | Exact CID/listener ownership, acquisition/release and evidence-routing contracts for identical concurrent suite selections; physical-host launch-resource scope. Reuse existing resources and kernel guarantees wherever they suffice; do not assume a new allocator service. |
| Q4 — ordinary execution routing | Close existing direct-command physical-host mutation paths while preserving selected command meaning and existing per-group freshness. Identify the exact treatment of build/Sim, guest tests, direct-native launcher tests, shell and maintenance; no invented CLI surface is accepted. |
| Q5 — evidence applicability | Reconcile the existing no-nesting native evidence rule with specifically accepted nested-boundary cells by naming the subject kernel/launcher and claim for each. Keep required direct-native proof and approved nested coverage. A general native-to-nested substitution is forbidden. |

For each semantic failure that a caller branches on, DESIGN pins the exact approved representation and mapping at the relevant shell/CLI/Rust interface; an existing generic carrier or a diagnostic string alone is insufficient. Contract-shaping native facts must be verified during DESIGN before acceptance. Any genuine conflict requiring weaker isolation or native evidence must return to the user; no such weakening is approved here.

Consulted: repository `AGENTS.md`, `CLAUDE.md`, relevant design/development/testing/verification/workflow/naming rules; `docs/product/jobs.yaml`, vision, existing platform-engineer personas and `trust-the-sim` journey; runner/suite sources cited above. `docs/project-brief.md`, `docs/stakeholders.yaml`, feature DISCOVER/DIVERGE artifacts: absent. The existing stopped DELIVER work and dirty files remain untouched.

## Wave: DESIGN / [REF] Status and authority

DESIGN proposal, 2026-11-08. **PROPOSED; implementation, a roadmap and later waves are not authorised.** The original service DELIVER remains stopped. D1–D5 above are the accepted user directions; P1–P5 below are the concrete new contracts requiring approval under `.claude/rules/design.md`.

Exact Linux v7.0 sources have been recovered from the official kernel.org repository, with the tag peeled to the checkout's exact gitlink commit. Native primitives, real paired GuestKit execution, selected-body routing and exact statuses have run. The initial 224-window allocator proposal is removed: the kernel's existing local-mode vsock namespace provides the needed owner and lifetime. No allocation daemon, slot registry, guest pool, outer guest per whole invocation or new product runtime API is proposed.

[Native evidence and current validation disposition](spike/findings.md), [exact source provenance](design/kernel-source-provenance.json), [closed placement inventory](design/placement-inventory.md). All contract-shaping source, inventory and bounded native construction work described here is complete. The native fixture prerequisite was isolated and its minimal corrected construction validated without changing a production test body. Material decisions remain proposed; user approval and independent design review are the remaining handoff gates.

## Wave: DESIGN / [REF] DDD list

| ID | Concrete proposed decision | Status | Main alternative / impact |
|---|---|---|---|
| P1 | One canonical Git-worktree identity owns remote source/build state. Use checkout-qualified branch fallback for an ordinary clone; bind overrides to the full identity; leave legacy caches intact and sync into a fresh identity. | PROPOSED | Branch/basename-only identity retains alias cases. Automatic adoption needs a migration protocol this change does not need. |
| P2 | Existing durable services share the physical-maintenance barrier and exclusively own their checkout. Ordinary owner records are invocation-specific; shell/provisioning and explicitly exclusive commands retain box-wide exclusion. | PROPOSED | Dropping the flock alone leaves singleton ownership and maintenance admission wrong. |
| P3 | Use the existing kernel local-mode vsock network namespace as the launcher resource boundary. Keep current numeric CID bands, actual bound ports, production VMM backend and fresh guest-group machinery. Scope ephemeral files by run ID. | PROPOSED | A userspace CID allocator adds unnecessary state/lifetime machinery and still does not isolate listeners. |
| P4 | Reuse nextest's per-test run-wrapper facility and the closed actual placement inventory. Preserve the selected binary, arguments, retry/profile/reporting owner and actual result. Keep arbitrary physical-host commands at an explicit exclusive boundary. | PROPOSED | A persistent guest per group would need an additional cross-process execution broker; replacing nextest with a batch-result interpreter would change its command semantics. |
| P5 | Invocation completion requires the existing systemd owner to end descendants and clean only that run's ephemeral roots. Keep actual command terminal fields; retain a separate completion receipt for repeatable read-only collection. | PROPOSED | A parent process exiting does not prove its guest/image resources have ended, and collection cannot perform cleanup. |

P4 has two visible consequences that must be surfaced: the proposed `metal run --exclusive -- COMMAND` flag preserves arbitrary privileged maintenance/probes with the old exclusive authority; ordinary raw `cargo test`/arbitrary shell chains are not silently interpreted. Direct nextest instantiates a fresh existing group for each selected kernel attempt; the explicit suite retains its established batched groups. These are material compatibility and guest-lifetime choices, not parser or fixture details to accept silently.

## Wave: DESIGN / [REF] Component decomposition and reuse

The following are logical component contracts, not requirements to split modules or prescribe private signatures. `bounded-change` names the entire permitted effect universe; its external complement, including every other invocation, must be preserved.

| Component / existing paths | Change | Owner and Contract Shape | Declared delta and bounded universe |
|---|---|---|---|
| Identity encoding/key derivation in `xtask/src/main.rs` / `infra/metal/bootstrap.sh` | EXTEND | Local resolver; **pure-function** | Return identity/key/validation result from supplied bytes; no observable state delta |
| Git/environment/SSH discovery in those adapters | EXTEND | Calling controller; **bounded-change** | Read actual Git/remote facts through this controller's subprocesses, descriptors and output; only those transient process resources may change |
| Source transfer in `bootstrap.sh` / `xtask/src/metal_pull.rs` | EXTEND | Checkout-lock owner; **bounded-change** | Update the single bound checkout's source, standalone Git copy, binding and source marker; pull updates only explicitly named local paths. Exclude target from source deletion; preserve other checkout/source/artifact/evidence complements |
| Durable registration/admission/command/finalizer in `lease-holder.sh` | EXTEND | Existing systemd invocation owner; **bounded-change** | Its unit/process tree, held locks, token-matched owner records, durable run records/logs and exactly its P5 ephemeral roots; shared/exclusive barrier changes are limited to its own held descriptor |
| Collection in `lease-holder.sh` | EXTEND | Existing collector; **bounded-change** | Its own observation descriptors, transient validation files and output only; delta of retained records, source, command processes and kernel test state is empty. Collection never cleans or rewrites them |
| `infra/metal/native-preflight.sh` | EXTEND | Existing qualification process; **bounded-change** | Its output, descriptors and already-existing short-lived qualification KVM/probe resources; no retained kernel/test/source change. Read this invocation's source and owner, not a singleton ordinary owner |
| Proposed `infra/metal/test-wrapper.sh` and nextest execution configuration | CREATE bounded adapter / EXTEND | Existing nextest attempt owner; **bounded-change** | The selected exec chain, inherited descriptors and actual output/status; read only placement and owned context, then execute exactly the assigned context. No guest/dispatcher dependency for host-pure bodies |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` and `src/{guest_kit,guest_lane,vm_fixture}.rs` execution | EXTEND | Existing disposable group/production VMM adapter; **bounded-change** | One existing group's guest kernel and processes, invocation-owned VMM/namespace/held sockets/images/directories, then cleanup. Initial physical-host effects stay within P3's finite allow-set; selected body once, other invocation complement preserved |
| `xtask/src/guest_kit_prepare.rs` and existing fixture preparation | EXTEND | Existing checkout preparation owner; **bounded-change** | Its checkout's target/module-build/staged ELF outputs and builder process resources; system kernel inputs are read-only, no module loading or another checkout's artifacts |
| Native run-parent constructors in `crates/overdrive-host/tests/integration/{vsock_vmm_device,vmm_equivalence}.rs` | Bounded test-support fallout | Existing native fixture owner; **bounded-change** | Its short invocation-owned parent/image/launcher resources, including GID 4200/mode 0710 parent traversal; retain every native confinement/data assertion |
| Proposed `infra/metal/test-placement.json` | CREATE static data | **Non-executable** | Closed exact-identity placement declaration, not an effectful component or service |
| Existing `.claude/hooks/block-{bare-nextest,cargo-test}.ts` command guards | Bounded operator-message fallout after approval | Hook process; **bounded-change** | Its stdin/stdout and finite allow/deny JSON only; describe the actual test entrypoint and explicit administrative boundary, without granting raw-host test fallback or changing another invocation |
| `infra/metal/README.md` and applicable testing documentation | EXTEND after approval | **Non-executable** | Describe the actual ordinary/exclusive and native/guest contracts |

No production `overdrive-core` or `overdrive-host` public type, method, variant, trait, persisted product state, transport format or product CLI change is proposed. Reuse existing `GuestKit::provision(&Path)`, `GuestFile`, `GuestPeer::exec`, `GuestKitVm` and production `Vmm::create/terminate`. Test-support plumbing is DISTILL's to shape; it cannot alter the pinned driving/wire/ownership/error contracts below.

## Wave: DESIGN / [REF] Driving ports and exact identity — P1

Existing `metal sync`, `run`, `pull`, `collect` and `shell` remain. Existing `--no-sync`, `--no-sudo` and `OVERDRIVE_METAL_REMOTE_SUBDIR` remain. P4 proposes exactly one new user flag, `metal run --exclusive`; no other user command/flag/configuration is proposed.

For sync/run/pull, discover identity once before mutation:

1. Require a non-bare Git working tree. Resolve physical checkout root, absolute Git directory and absolute common Git directory. A failed/malformed/permission-denied query is an identity error, not branch fallback.
2. Git directory different from common directory means `kind=worktree`; identity bytes are the canonical absolute worktree-specific Git-directory bytes. Branch changes and detached HEAD do not change this linked-worktree identity.
3. Git directory equal to common directory means `kind=branch`; require an attached full symbolic branch ref. Identity bytes are physical checkout-root bytes, one NUL, then full ref bytes. Separate clones of one branch remain disjoint. A detached ordinary clone refuses; it does not use a directory-name guess.
4. `checkout_key` is full lowercase SHA-256 of ASCII `overdrive-metal-checkout-v1`, NUL, kind, NUL, identity bytes. Complete identity equality, not hash equality alone, authorises reuse.
5. Default remote path is `overdrive-worktrees/<checkout_key>` or `overdrive-branches/<checkout_key>`, relative to the physical remote login home. Workspace/branch spelling is diagnostic only. Moving Git metadata or an ordinary clone selects a cold identity; no implicit adoption follows.
6. An explicit override selects a directory, not a different owner. Require `[A-Za-z0-9._/-]+`, relative, without empty/`.`/`..` components or symlink components, below the physical remote home. Name its lock with full SHA-256 of the canonical absolute remote-directory bytes. Different spellings cannot create separate locks for the same directory.

`<remote-dir>/.overdrive-metal-checkout.v1` is excluded from rsync deletion. Ordered UTF-8 LF-terminated fields, exactly once:

```text
format=overdrive-metal-checkout-v1
kind=<worktree or branch>
checkout_key=<64 lowercase hex>
identity_b64=<canonical RFC 4648 base64 of the exact identity bytes>
```

A branch payload has exactly the one delimiter NUL described above; paths and refs cannot contain NUL. Bind only an empty/new directory under its exclusive checkout lock. A non-empty unbound directory, malformed binding or different full owner refuses. A successful sync preserves this binding and atomically publishes the existing source marker only after transfer. Run `--no-sync` requires both binding and the existing exact `commit/workspace/source_digest` marker. Pull takes the same lock and binding but does not require the current local dirty digest to equal the pre-run digest: generated evidence is what it retrieves. Collect uses target plus validated run ID and does not require the current checkout/branch to match that run.

Migration is explicit and non-destructive: retain existing sanitised-branch directories and all old run records. New defaults start cold and need sync. An override pointing at an unbound legacy directory refuses; choose a fresh directory rather than silently adopting another checkout's artifacts.

## Wave: DESIGN / [REF] Driven ports, maintenance and durable ownership — P2

| Driven port | Reused adapter | Exact promise |
|---|---|---|
| Source transfer | Existing SSH/rsync | One bound checkout under its lock; ordinary sync/run never install host packages. Missing remote rsync maps to `runner_preparation_failed`/75 as specified below; provisioning owns installation. |
| Physical maintenance admission | Existing `/run/lock/overdrive-metal-shared.lock` | Ordinary durable service takes shared flock; provisioning, shell and `run --exclusive` take exclusive flock. Independent ordinary commands do not exclude each other. |
| Checkout exclusion | Existing tree flock | Durable service acquires it before authorising controller rsync, holds it through the selected command, then releases. The controller no longer releases/reacquires it around START. Sync/pull use this same lock. |
| Durable execution | Existing `overdrive-metal-run-<run-id>.service` | Keep `Type=exec`, `KillMode=mixed`, actual command/result handling and controller detachment. Add only P5's bounded finalizer. |
| Native qualification | Existing preflight | Keep literal x86_64, non-virtualised host, CPU extensions, KVM API/create-VM, cgroup v2, VMM and selected guest-input checks. |
| Launch | Existing `CloudHypervisorVmm` | Same renderer, uid/gid 4200, supplementary device groups, no-new-privs, launch seccomp and Landlock. The VMM alone opens vhost-vsock; CID-NOCLAIM stays intact. |

Missing remote `rsync` is a source-transfer **preparation** failure, never a selected-command result or native-probe failure. A transfer-requiring ordinary `sync`, `pull` or pre-START `run` emits `OVERDRIVE_METAL_RUNNER_ERROR error=runner_preparation_failed prerequisite=rsync` and exits 75. For `sync`/`pull` there is no durable registration or new run record; release their checkout lock. For a registered run, remain in `preparing`, send the already-declared `ABORT error=runner_preparation_failed`, and never send START. Retain `terminal.v1 kind=runner error=runner_preparation_failed` using its existing ordered format, then perform P5 finalization. With valid completion, collection emits the same `runner_preparation_failed` code and returns 75; it does not replace it with `launch_aborted` or a command terminal. `prerequisite=rsync` is immediate diagnostic context, not a new terminal field. P5's missing-terminal (70) and malformed/missing-completion or cleanup-failure (74) precedence still applies. `--no-sync` does not require unused rsync. Ordinary operations never install it; only explicit exclusive provisioning retains that responsibility. This replaces the current automatic-install branch at `bootstrap.sh:518-525`; it is a proposed contract, not a claimed implemented failure observation.

Lock order is maintenance shared/exclusive → checkout exclusive. Sync/pull take checkout only and perform no physical-host maintenance. One existing admission deadline covers waits; do not silently add a full new timeout for each lock. A different worktree's ordinary run is never an exclusive barrier holder.

Ordinary owner metadata moves to `<run-dir>/owner.v1`; ordinary runs never overwrite the singleton physical-maintenance owner. Preserve the current ordered owner fields (`pid`, `started_at`, `action`, `scenario`, `workspace`, `commit`, `token`, `run_id`, `unit`, `source_digest`); `action=run`, token and run ID are this invocation's 24 lowercase hex value. The checkout owner file retains the standard holder's seven ordered fields (`pid`, `started_at`, `action`, `scenario`, `workspace`, `commit`, `token`), with `action=run` and token equal to this run ID while the durable service owns the checkout. Exclusive runs also publish the token-matched singleton owner for existing maintenance diagnostics. Native preflight receives the invocation owner path through the existing `OVERDRIVE_METAL_OWNER_PATH` and verifies this token plus bound source. Cleanup removes only records whose token matches its owner.

The private durable launch has eighteen positional values: original values 1–15 unchanged, then exactly values 16–18: `TREE_LOCK_PATH`, `TREE_OWNER_PATH`, `EXECUTION_CLASS` (`ordinary|exclusive`). All supported physical callers use the canonical maintenance barrier; existing path substitutions remain test-support fixture inputs, not an operator permission to create a second physical admission domain.

The acquired marker means the service owns admission and checkout exclusion, so only its controller may transfer source. Keep one controller START, no replay. Accepted control lines are `START`, `ABORT`, or `ABORT error=<checkout_identity_conflict|source_mismatch|runner_preparation_failed|preflight_failed>`. A tagged ABORT persists that exact approved runner error; malformed control retains `launch_aborted`. EOF before START retains `controller_eof_before_start`. After START, controller stdin/stdout/stderr detach as today; losing the local route never cancels the durable command.

Keep existing phase shape and literals. `waiting_for_lease` in a new record means admission before this run owns both locks; it may wait for maintenance or its own checkout. `preparing` authorises source/tool preparation and preflight; `running` retains the existing launched-command/detached-controller promise. It is not guest READY or test success. Invocation filesystem/context preparation belongs before START. A missing guest dispatcher never triggers an on-demand Cargo build from a nextest test wrapper; keep the existing fixture preparation owner.

New `metadata.v2` uses `format=overdrive-metal-run-v2`, retains every existing ordered field, and appends `execution_class=ordinary|exclusive`. New runs write only `metadata.v2`; old `metadata.v1` remains unchanged. The collector accepts exactly one matching filename/format pair; both files or a mismatched format is `run_artifact_io`, not a precedence guess. Existing `execution_mode=privileged|login` stays distinct: lock authority is not Unix identity. Copy the checkout binding to `<run-dir>/checkout.v1`. Keep actual command `terminal.v1` fields and 0..255 code unchanged. P5 defines the additional completion receipt. The collector accepts old v1 records under their original rules; it never treats an incomplete v2 record as legacy. Both ordinary callers in the concurrent acceptance capture must use the updated runner protocol. Older source branches sending the legacy exclusive protocol remain exclusive and are not credited as ordinary v2 execution; their barrier still excludes physical maintenance/ordinary overlap safely. This requires no shared-tool daemon or automatic edits to another worktree.

Exclusive maintenance also waits, within its admission budget, for already-admitted transient units in preparing/running to finish their existing systemd stop/cleanup. A main process releasing a descriptor is not proof all launcher descendants ended. Waiting registered ordinary units that have not acquired the shared barrier cannot block the exclusive holder. Reuse the existing unit-liveness reader; add no recovery controller or automatic stale-run scavenger.

## Wave: DESIGN / [REF] Kernel resource boundary and finite lifecycle — P3

The numeric contracts remain `GUEST_CID_BASE=0x01000000`, `GUEST_CID_SPAN=65536`, all existing `CidBand` fields/constants and `raw/cid` signatures. **There is no run CID window, slot environment variable, hash allocation, preclaim device or claim registry.**

For an ordinary guest/native wrapper, before any selected body or guest launch:

1. Create a fresh anonymous host network namespace in the wrapper process.
2. Set only that fresh namespace's `net.vsock.child_ns_mode=local`.
3. Create the child network namespace and verify its read-only `net.vsock.ns_mode` is `local`.
4. Bind the actual listeners and launch the VMM in this local context. Never write the initial physical namespace's mode. The temporary parent need not remain held after the child exists.

Two local namespaces can hold identical CID/port values concurrently. The kernel matches CID plus permitted namespace, claims atomically, and holds the VMM opener's namespace until device release. A surviving VMM therefore keeps its old context; a new invocation creates a different context rather than guessing a free CID. The actual paired native GuestKit proof used CID `16785415` in both instances, observed overlapping guest-ready intervals and different guest boot IDs, and retained the second control path after the first ended. The initial namespace's modes and observed module/nftables/BPF/routes/test-cgroup complement were unchanged.

Exact source at v7.0 / `028ef9c96e96197026887c0f092424679298aae8`: `drivers/vhost/vsock.c:74-88,688-690,753-824`; `include/net/af_vsock.h:279-323`; `net/vmw_vsock/af_vsock.c:781-832,2824-2864,2923-2932`. [Recovered files, official URLs and SHA-256](design/kernel-source-provenance.json). No different kernel version was substituted.

Use the existing kernel-assigned listeners and actual bound ports. In `guest_lane`, use the beacon from `HostListeners::bind` in the existing `overdrive.transport=` token and accept on that same held listener; remove the separate fixed pre-cut bind. Current init already dials `token.ports.beacon` (`overdrive-init/src/main.rs:585-590`); no product init/wire change is needed. Peer CID and existing launch/session attribution remain unchanged.

| Resource | Owner | Lifetime / cleanup |
|---|---|---|
| Anonymous local vsock namespace | This wrapper and its VMM/socket references | Kernel reference lifetime; no named namespace file or stale-ID reuse |
| CID and vsock listeners | Actual VMM device / held listener descriptors in that namespace | Existing VMM termination/reap and descriptor close; no independent vhost holder |
| Guest kernel/module/selected ELF | Existing group fixture | Exact selected source artifacts, one fresh instance of its existing group |
| Guest images and fixture work | Invocation-scoped staging root | Existing normal Drop cleanup; P5 handles supported process termination |
| VMM runtime/API/console | Invocation-scoped short runtime root | Existing production adapter plus P5 cleanup after descendants end |
| Build target/module artifacts | Bound remote checkout | Checkout lock and existing source-scoped paths; never a different worktree's target |
| Logs, metadata, terminal/completion | Durable invocation | Retained without automatic pruning; read-only repeat collection |

For Metal context, `default_staging_root` selects `/srv/vm/metal/<run-id>` and host VMM runtime work stays below `/run/ovd-gk/<run-id>`. Outside Metal, existing local/Lima behaviour remains. The wrapper validates the service-generated run ID before any path operation; a bad context refuses before a body starts. The staging base `/srv/vm/metal`, runtime base `/run/ovd-gk` and their run children are root-owned with group 4200 and explicit mode 0710; set the final mode explicitly rather than relying on mkdir mode through the inherited umask. They grant the existing VMM group traversal, not write access. Preserve per-launch image permissions. Native fixture constructors must also give their supplied run parent group-4200 traversal before `Vmm::create`, matching the existing `GuestKit::boot` construction contract; assigning the per-allocation child to uid 4200 cannot repair an untraversable parent. Keep actual API paths below Linux's pathname socket bound; the proposed root keeps the longest existing native identity path below 108 bytes even with seven-digit PID and three-digit fixture counter. Do not replace it with a long worktree/branch path.

The selected command's temporary root is `<remote-dir>/target/metal-run-tmp/<run-id>` with access matching the existing privileged/login mode. `CARGO_TARGET_DIR` is this checkout's target; supported mutation scratch copies stay inside this invocation's owned temporary universe and retain existing source-artifact overrides. External shared target/module/evidence overrides refuse ordinary admission rather than mixing worktrees.

The physical-host allow-set is existing unit/cgroup/process confinement, native KVM/vhost handles, actual held sockets, these anonymous launcher namespaces and named filesystem roots, and existing invocation-owned image-preparation resources. It excludes physical-host test-owned module/nftables/BPF/route/test-cgroup mutations. Kernel bodies execute in disposable guests. A source-derived host crypto/file/Sim test is distinct from a test exercising those kernel owners; [the closed inventory](design/placement-inventory.md) records that distinction.

## Wave: DESIGN / [REF] Ordinary command and test execution — P4

Recommended exact new flag: `cargo xtask metal run --exclusive [--no-sync] [--no-sudo] -- COMMAND [ARG ...]`; bootstrap receives `--run --exclusive`. It is valid only with run: other public xtask subcommands retain Clap's parse error/status 2; the direct bootstrap interface reports `invalid_mode`/64 for that invalid combination. It preserves the old arbitrary-command authority, durable start/result/collection and box-wide exclusion for explicit physical-host maintenance or specialised native work. Existing provisioning and `metal shell` remain exclusive. Ordinary is the default; an unsupported command never silently becomes exclusive or executes privileged on the host.

Ordinary admission is a finite list of existing structured entrypoints, not an arbitrary shell interpreter:

| Existing ordinary entrypoint | Execution |
|---|---|
| `cargo build/check/clippy/fmt ...`, `cargo test --doc ...` | Existing build/source/file/CPU operation; no integration-test body is run by a guessed command interpreter |
| `cargo xtask guest-kit-prepare`, existing BPF/module **build** and lint/model commands | Existing setup/tool orchestration; no `kmod-load` |
| `cargo nextest run ...`, existing mutation orchestration using nextest | nextest owns selection, profiles, ignored policy, retries, concurrency, reports and aggregate status; the mandatory wrapper owns only placement |
| Existing `cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite ...` | Existing explicit group order, selectors and canonical body-count rules |
| Existing declared pure Sim program entrypoint | Same host program and arguments; no guest or unrelated-run execution exclusion. Current `dst` parser/default catalogue excludes the two socket-owning Gateway Application variants; see the source/reachability qualification below |
| `cargo test` integration bodies, arbitrary privileged executables/shell chains, `cargo xtask kmod-load` | Ordinary refusal; use the declared test entrypoint or the explicit exclusive administrative boundary as appropriate |

Exact ordinary argv grammar: the executable token is literal `cargo`, optionally preceded by literal `env` or `/usr/bin/env` and zero or more permitted `KEY=VALUE` tokens. No env options, shell `-c`, shell metacharacter interpretation or alias expansion is performed. After `cargo`, accept exactly: `build`, `check`, `clippy`, `fmt`; `test` only when Cargo's argument segment contains `--doc`; `nextest run`; `xtask` followed by one of `guest-kit-prepare`, `bpf-build`, `bpf-clippy`, `bpf-unit`, `kmod-build`, `dst-lint`, `cloexec-lint`, `conformance-lint`, `yaml-free-cli`, `quint`, `mutants`; or `run` selecting exactly (`overdrive-testing`, `overdrive-fwd-guest-suite`), (`overdrive-sim`, `dst`) or (`overdrive-control-plane`, `openapi`) by Cargo package/bin options. Package options accept `-p VALUE`, `--package VALUE`, `--package=VALUE`; bin accepts `--bin VALUE` or `--bin=VALUE`. Conflicting/missing package/bin selectors refuse. The remaining arguments go unchanged to that existing tool; do not invent an allowlist of every nextest report/filter option. Root Cargo aliases and other forms remain available through the explicit exclusive boundary rather than being guessed ordinary commands.

The current `dst` boundary is qualified by its actual catalogue: `Invariant::from_str` searches `Invariant::ALL`, and the default harness also uses `ALL`. Both Gateway Application variants are absent pending activation (`overdrive-sim/src/invariants/mod.rs:731-875,958-971`, `src/harness.rs:309,744-752`). Their canonical `--only` strings therefore return the existing unknown-invariant status 1 before running a seed. The six current Rust acceptance identities that explicitly use those socket owners are instead classified `guest-group/cgroup-default`. A retained concurrent exact-test proof reproduces `AddrInUse`; a real CLI invocation rejects the selector. [Disposition and raw evidence](design/f1-disposition.md). No CLI argument is rewritten and no inactive invariant is activated. If that catalogue later activates a real socket owner, its ordinary program placement must be reviewed at that change; current typed Rust reachability must not be confused with current CLI reachability.

The exact optional env keys are `OVERDRIVE_BPF_NATIVE` (only literal `1`), `OVERDRIVE_FWD_GUEST_GROUP` (the existing eight group names or `all`), `OVERDRIVE_FWD_GUEST_EXACT`, the thirteen `PROPTEST_*` keys declared by the pinned Proptest 1.11.0 configuration (`PROPTEST_CASES`, `PROPTEST_DISABLE_FAILURE_PERSISTENCE`, `PROPTEST_FORK`, `PROPTEST_MAX_DEFAULT_SIZE_RANGE`, `PROPTEST_MAX_FLAT_MAP_REGENS`, `PROPTEST_MAX_GLOBAL_REJECTS`, `PROPTEST_MAX_LOCAL_REJECTS`, `PROPTEST_MAX_SHRINK_ITERS`, `PROPTEST_MAX_SHRINK_TIME`, `PROPTEST_RNG_ALGORITHM`, `PROPTEST_RNG_SEED`, `PROPTEST_TIMEOUT`, `PROPTEST_VERBOSE`), `RUST_BACKTRACE`, `RUST_LOG`, and `NEXTEST_PROFILE`. Values other than the two finite BPF/group selectors are forwarded unchanged for their existing tool to validate; they must be valid UTF-8 argv strings. Duplicate keys and every other key refuse ordinary admission. The service owns and overrides its run/dispatch/artifact context, not the caller's env prefix. Existing values needed by a guest body (`PROPTEST_*`, `RUST_*` above and nextest's identity/profile/attempt metadata) are explicitly passed with the existing staged `env` program through `GuestPeer::exec`; do not silently replace a requested property budget or retry context with the guest PID 1's defaults. Guest HOME/PATH and fixture resource paths remain the existing group environment, not a copy of host filesystem assumptions.

Refuse caller execution/config replacements: `--config`, `--config-file`, `--tool-config-file`, `--user-config-file`, debugger/tracer replacement (`--debugger`, `--tracer`), or a configured Cargo runner overriding Metal's execution adapter. Ordinary archive/build-reuse options (`--archive-file`, `--archive-format`, `--extract-to`, `--extract-overwrite`, `--persist-extract-tempdir`, `--cargo-metadata`, `--binaries-metadata`, `--workspace-remap`, `--target-dir-remap`, `--build-dir-remap`) refuse: they do not establish current checkout provenance. Ordinary `--manifest-path` and `--target-dir` (including `=VALUE`) must canonicalise under the bound checkout or this run's owned scratch universe before execution; external shared target/module/evidence paths refuse. Other normal selector/retry/profile/report arguments retain their owner and exact values. The checked-in nextest configuration must keep the mandatory wrapper on every profile; a source configuration replacing it is `execution_override_refused`, not a raw-host fallback.

Use nextest's existing `run-wrapper` configuration, with `target-runner=within-wrapper`. A small checked-in exec adapter looks up the declared placement only in ordinary Metal context; outside ordinary Metal context (non-Metal or explicit exclusive mode) it `exec`s the original argv (including any configured runner) unchanged. Host-pure bodies exec directly with no guest-dispatcher preparation dependency. Native-launcher bodies establish their native resource context and exec the selected body. Guest contexts call the existing prepared suite dispatcher; extend the existing `guest-kit-prepare` step to incrementally build that host executable alongside its current artifacts each time the declared lane setup runs. Do not add an on-demand Cargo build inside a wrapper to replace a missing dispatcher. Existing fixture-internal preparation keeps its existing owner. This preserves actual nextest compile-failure status instead of compiling product dependencies as a new controller prerequisite.

The service injects exactly the following internal context fields: `OVERDRIVE_METAL_RUN_ID` (24 lowercase hex), `OVERDRIVE_METAL_EXECUTION_CLASS` (`ordinary|exclusive`), `OVERDRIVE_METAL_SOURCE_ROOT` (the canonical bound remote checkout), and `OVERDRIVE_METAL_TEST_DISPATCHER` (the prepared existing suite executable under that checkout's target). The exec adapter uses the captured absolute dispatcher path even when mutation orchestration runs from an owned scratch copy; it does not infer another checkout from compile-time paths. Missing dispatcher for a guest context is `lane_not_prepared`, never raw-host execution.

Proposed existing executable entry: `overdrive-fwd-guest-suite --nextest-wrapper <test-binary> [<libtest-arg> ...]`. Exact identity is nextest's `NEXTEST_BINARY_ID` plus `NEXTEST_TEST_NAME`; retain `NEXTEST_ATTEMPT` and other tool metadata. The adapter must use the actual Cargo-reported executable argument, not a stale pre-staged test ELF. Existing `GuestFile` carries it and its library closure into the matching group. Also stage any nextest-reported non-test helper executables (`NEXTEST_BIN_EXE_*`) at their reported guest paths and forward their matching tool metadata; a compiled helper path must not become a missing-file failure merely because the body moved into its guest. Existing `GuestPeer::exec` returns its actual code to nextest. Preserve nextest's own aggregate status: a body returning 255 is a failed attempt and nextest returns its actual 100; a directly supervised command returning 255 remains 255.

The static placement data has `format=overdrive-metal-test-placement-v1` and a mapping `placements[binary_id][exact_test_name]` to exactly one descriptor:

```text
{"kind":"host-pure"}
{"kind":"native-launcher"}
{"kind":"guest-group","group":"probe|core|unload|ulp|mtls-relay|cgroup-default|stress|nested-boundary"}
{"kind":"guest-lane-operator","fixture":"<existing exact init fixture identity>"}
{"kind":"guest-lane-owner","fixture":"<existing exact init fixture identity>"}
```

The pipe-separated group line denotes its eight allowed literal values. For both guest-lane descriptors, `fixture` is the exact Rust identity `integration::in_guest_lane::<function>` in fixed binary `overdrive-init::integration`; the initial leaf names in the inventory expand with this prefix. It identifies the existing setup/host-script context, not an extra libtest body to execute or credit. Unknown fields/kinds/groups, duplicate identities, ambiguous placement or a missing identity refuse before body execution. There is no absence-means-host rule. The 4,640-row source-reviewed inventory is the initial complete set, not a requirement to copy rationale text into runtime data. Data compaction must retain exactly the same partition. Guest-cell descriptors reuse the already-authored operator/owner context and host script; they execute the selected cell once, not an extra unselected test body. Test-support extraction is DISTILL-owned.

A direct kernel nextest attempt, including a retry, gets a fresh instance of its existing group. The explicit suite keeps its existing batching; no canonical group-count credit is awarded to a focused wrapper attempt. Reuse the existing `host-kernel-shared` per-nextest group (`max-threads=1`) as the finite guest-capacity owner; extend assignment to newly guest-routed fixed-port Gateway fixtures and raw init cells. Do not add a scheduler, persistent guest pool or box-wide ordinary execution lease.

## Wave: DESIGN / [REF] Completion and read-only collection — P5

Keep actual command `terminal.v1` publication in the existing supervisor. Add an existing-unit `ExecStopPost` finalizer, not a new service or recovery protocol. Its code/arguments are captured for that invocation, independent of later source sync. Private entry contract: `lease-holder.sh --finalize RUN_ID RUNS_ROOT`. It uses validated run ID and retained metadata to clean only:

- `/srv/vm/metal/<run-id>`;
- `/run/ovd-gk/<run-id>`;
- `<recorded-remote-dir>/target/metal-run-tmp/<run-id>`.

Normal fixture Drop/terminate remains first owner. The finalizer runs after systemd's existing mixed-mode stop ended prior descendants; it then removes remaining owned ephemeral roots and atomically retains `completion.v1`. Missing roots are already clean. No glob, other invocation's path, source checkout, Cargo target, kernel module or host policy is removed.

Exact ordered LF records:

```text
format=overdrive-metal-completion-v1
run_id=<24 lowercase hex>
kind=complete
finished_at=<existing RFC3339 UTC shape>
```

or:

```text
format=overdrive-metal-completion-v1
run_id=<24 lowercase hex>
kind=runner
error=cleanup_failed
root=<guest_staging|vmm_runtime|command_tmp>
finished_at=<RFC3339 UTC>
```

A v2 collector waits for the unit to stop before emitting final completion. With valid terminal plus complete receipt, return the actual saved command/runner result. Cleanup failure returns typed runner failure 74; the actual command terminal remains retained. Missing/malformed completion after a stopped unit is `run_artifact_io`, never successful cleanup. Missing command terminal after supervisor loss remains `supervisor_lost` 70 even if cleanup ran; no command result is reconstructed from systemd. Existing malformed-phase precedence remains. Old v1 runs retain original collection rules and do not require a completion record. Collection never performs cleanup or writes any record; repeated collections agree.

## Wave: DESIGN / [REF] Native evidence applicability

Q5 is resolved by actual subject and existing fixture ownership, not by a general nesting exception. [Every placement](design/test-placement-inventory.json) and [subject inventory](design/placement-inventory.md) identify the boundary.

| Actual subject | Execution / credit |
|---|---|
| Production native VMM launch, vhost device lifetime and launch confinement | Keep direct physical x86_64/KVM bodies; launcher namespace is resource confinement, not another hypervisor. Nested results do not replace this proof. |
| Existing host flow/beacon accept seams against a scripted guest | Direct native controller and one first-level guest; same production adapters and peer identities |
| Forwarder/module/BPF/nftables/routes/cgroups/keyring kernel correctness | The actual disposable guest Linux kernel is the SUT; physical kernel is not mutated to manufacture the test |
| Existing init operator/owner cells | Retain their existing first-level guest and exact host scripts |
| Specifically named `nested-boundary`, STEP 14 and VM-cohort correctness cells | Keep their approved nested composition/correctness assertions, exact counts and no-benchmark-credit limits; never call them direct-native boot evidence |
| Pure/Sim/private-file and build checks | No native driver/kernel credit, even when invoked on the Metal box |

The no-nesting sentence in `testing.md:1795-1816` governs the authoritative driver boot surface. `netns-density-295/deliver/vm-cohort-support.md:82-101` and its review preserve specifically named nested correctness fixtures. Neither grants a blanket substitution. No accepted native requirement, test assertion, zero-state invariant, error distinction or stopped-feature design is relaxed here. The observed native fixture failure was reproduced in the original initial namespace, traced to its caller-supplied parent permissions, and the copied construction passes in both contexts with only group-4200/mode-0710 parent preparation. Its 128 MiB declaration and original assertions are retained; do not repair that fixture by nesting it or weakening its oracle.

## Wave: DESIGN / [REF] Typed failures

Shell markers are typed finite fields, not diagnostic prose. `OVERDRIVE_METAL_RUNNER_ERROR error=<code>` identifies runner failures; `OVERDRIVE_METAL_TEST_ERROR error=<code>` identifies placement/fixture failures. Existing raw OS/tool diagnostics add context only.

| Semantic failure | Exact code / payload | Status and retention |
|---|---|---|
| Invalid Git discovery or detached ordinary clone | `checkout_identity_invalid` | 64 before remote mutation; no fallback guess |
| Invalid override | `remote_subdir_invalid` | 64 |
| Unbound non-empty / differently bound / malformed checkout | `checkout_identity_conflict` | 78; tagged ABORT persists same code if registered |
| No-sync binding or source mismatch | `source_mismatch` | 78; registered runner terminal same code |
| Maintenance barrier / admitted predecessor cleanup deadline | `maintenance_timeout` | 75; runner terminal |
| Own checkout deadline | `checkout_timeout` | 75; runner terminal |
| Invocation-owned filesystem/context or source-transfer prerequisite preparation failed | `runner_preparation_failed`; missing rsync immediate `prerequisite=rsync` | 75; pre-registration no record; registered tagged ABORT, retained runner terminal and P5 finalization; valid collected completion returns same code/75 |
| Existing native/source/owner prerequisite failed | `preflight_failed`, immediate `probe=<arch|virtualization|cpu|kvm_device|kvm_api|cgroup_v2|vmm|guest_kernel|guest_rootfs|owner|source>` | 78; tagged ABORT persists preflight failure; no native evidence |
| Ordinary command / bypassing config / foreign artifact scope refused | `unsupported_ordinary_command`, `execution_override_refused`, `artifact_scope_mismatch` respectively | 64 before START; no implicit exclusive or raw-host fallback |
| Invalid run mode (`--exclusive` without run) | Public CLI: existing Clap unknown-argument parse error; bootstrap: `invalid_mode` | CLI 2; direct bootstrap 64, both before registration |
| Missing/invalid dispatcher context | test `invalid_run_context`, `field=<run_id|execution_class|source_owner>` | wrapper 78 before body |
| Invalid, duplicate or missing placement | test `placement_invalid`, `placement_ambiguous`, `test_unclassified` respectively | wrapper 78 before body |
| Namespace operation fails | test `namespace_setup_failed`, `step=<parent|child_mode|local|verify>`, `errno=<positive decimal|none>` | wrapper 70 before body; no global-CID fallback |
| Required context/dispatcher/guest input absent | test `lane_not_prepared` | wrapper 78; no body/evidence |
| Existing image/VMM/peer execution failure | test `guest_execution_failed` plus original typed `GuestKitError` cause | wrapper 70; nextest retains its actual aggregate status |
| Finalizer cannot remove owned root | `cleanup_failed`, completion root enum above | collector 74; actual command terminal retained |
| Corrupt/missing retained artifact | Existing `run_artifact_io` | 74, never promoted to result |
| Invalid ID / unknown run / supervisor loss | Existing `invalid_run_id` / `run_not_found` / `supervisor_lost` | Existing 64 / 66 / 70 |
| Existing ABORT/EOF/launch/log errors | Existing exact runner error values | Existing 75 / 74 mappings; retain v1 compatibility |
| Actual selected command/body status | Existing command terminal / peer status | Actual 0..255 code; nextest owns its aggregate mapping and retries |

The v2 terminal validator adds exactly the registered admission/preparation/preflight codes above; unknown values remain malformed. No new product Rust error envelope/variant is needed. A command result numerically equal to a runner status remains distinguishable by typed record/marker. Namespace and placement failures belong to the test-execution adapter, not product readiness/health.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

| State / owner | Exact promise | Allowed gates / unaffected states |
|---|---|---|
| Local printed run ID | Identifier selected | Does not prove registration or start |
| Durable registered / existing service | Initial records retained | May wait for maintenance/own checkout; no product lifecycle meaning |
| Preparing / existing service | Admission and checkout held; controller may transfer/prepare | Source/tool/native checks gate only run admission |
| Running / existing supervisor | Selected command process exists; controller descriptors detached | Does not mean guest READY, test pass or cleanup complete |
| Command terminal / existing supervisor | Actual command result and finalised log retained | Does not manufacture a result after supervisor loss |
| Completion / existing unit finalizer | Prior descendants ended and owned ephemeral roots removed, or exact typed failure | Gates v2 collection completion; never gates product Running/Stable/readiness/health |

**G1 identity:** local resolver → bootstrap → exclusive checkout → full binding → transfer/source marker → preflight → START. Counterexample: a linked worktree's branch switch must not select another cache. Native/local identity cases and paired real sync are the evidence boundary.

**G2 admission:** existing durable service takes shared/exclusive maintenance then checkout. One timeout budget; unknown/malformed ownership refuses. Counterexample: another worktree's pure Sim command cannot hold an ordinary exclusive barrier. Native shared/exclusive primitive proof is held; paired new-run execution remains implementation verification.

**G3 placement:** nextest owns selection/retry/result; the existing group adapter owns where a selected body executes. Unknown/malformed/contradictory context refuses before body. Counterexample: absence from a guest list cannot grant host execution. Native actual body/status/paired-group proofs and the closed inventory cover the construction.

**G4 completion:** existing systemd stop → finalizer → complete/typed failure receipt → read-only collector result. Late controller output and repeated collection cannot replay START, alter a terminal or reclaim another run. A late cleanup success cannot synthesise a missing command status. Native timeout construction must demonstrate this existing owner path; a hypothetical arbitrary future abort is not the premise.

No feature-enable flag, new control-plane ordering mechanism, recovery service, startup-probe framework, seeded Sim seam or product state is added. Legacy collection is the explicit compatibility case. All new test-support verification keeps required Contract Shape declarations and the repository's test/expectation boundaries.

## Wave: DESIGN / [REF] Technology, capacity and validation

Reuse Bash/OpenSSH/rsync, current Rust workspace, existing systemd transient services (**259** observed), native nextest **0.9.149** (local help/source inspection was 0.9.150), current production Cloud Hypervisor fork and observed Linux `7.0.0-38-generic`. No new proprietary dependency, runtime service or framework. Placement JSON is internal tool data, not a product/operator config interface.

The native resource proof ran two ready guests with identical CID concurrently and observed disjoint namespaces/boot IDs. Standard group envelopes remain 4,096 MiB each plus existing `MemoryPlan` reserves: two such groups require at least 8,192 MiB guest RAM plus reserve/launcher/build overhead. Existing focused 64-VM envelopes remain untouched; no paired density benchmark is added. Per-nextest existing max-one guest capacity and distinct invocation namespaces replace no resource scheduling service. No unlimited concurrency or speedup ratio is claimed.

Direct fine-grained nextest can select up to the inventoried 782 ordinary group bodies plus 11 special guest cells; fresh-per-attempt execution has real boot overhead. Use the existing explicit batched suite for its existing batched journey. This trade-off must be approved under P4; no invented pooling mechanism is hidden in implementation. Bodies classified host-pure/Sim get no guest and no new cross-run exclusion; the six real-socket Sim acceptance identities use their declared guest context.

Native DESIGN work used only the existing permitted runner, queued/retried without interrupting another workspace, built required existing prerequisites, listed actual tests without executing a broad suite, and ran bounded selected construction probes. [Evidence](spike/findings.md) records positive and negative attempts honestly. Outcome collision CLI ran successfully but checked zero outcomes; that is not semantic collision assurance. No new recurring lane, benchmark, sanitizer, mutation run or dashboard is requested. The required bounded admission/completion protocol model is recorded below.

## Wave: DESIGN / [REF] Formal protocol model

P2/ADR-0278 and P5/ADR-0281 change shared-owner admission and completion ordering, so the repository's formal gate applies. The permanent [runner protocol](../../../specs/quint/metal-run-isolation/runner_protocol.qnt), [manifest](../../../specs/quint/metal-run-isolation/checks.toml), [abstraction and assumptions](../../../specs/quint/metal-run-isolation/README.md) and [tool-written evidence](../../../specs/quint/metal-run-isolation/evidence/summary.md) cover **two invocation slots, same/different checkout and one exclusive maintainer**. They add no runtime protocol, product API or Sim seam. P1 identity validation, P3 kernel namespace guarantees and P4 placement are explicit input/adapter assumptions, discharged by the retained source/native evidence and schema/placement validation; Quint does not prove kernel behavior.

`runner_ok` exhaustively checks `Safety`, `OwnersEventuallyEnd` and `MaintenanceEventuallyResolves`. The recorded local check returned 0 with **18/18 expected outcomes** and current hashes passed `verify-evidence`: [complete validation](design/remediation.md#completed-validation). The progress claims assume weak fairness of enabled admission-deadline, preparation, command-exit and systemd-stop operations. They do not promise termination of an infinite selected command or hung storage/kernel. One hazard each removes P2's shared ordinary admission or P5's completed-owner collection gate and must yield the respective counterexample. There are no redundant witnesses, bounded copies or rejected-architecture checks.

Every projected accessor below is an existing shell/systemd/kernel observation, used by a test-local external driver; no new production accessor signature is proposed. The driver validates the pinned record schema before decoding fields. It does not compare its own ledger to the model.

| Model field | Production owner and exact observable accessor / projection |
|---|---|
| `runs[r].p` | Durable unit: `systemctl show overdrive-metal-run-<id>.service --property=MainPID,ActiveState,ControlGroup`, retained `phase.v1`/`terminal.v1`, and kernel locks at `/proc/<MainPID>/fdinfo/<fd>` with `/proc/<MainPID>/fd/<fd>` resolving the named barrier/checkout path. 0 = absent metadata and nonlive unit; 1 = registered `waiting_for_lease`, no barrier; 2 = shared barrier held, no checkout; 3 = `preparing` with both held; 4 = `running` main/selected command live; 5 = main ended but activating/active/deactivating unit still live; 6 = unit inactive/failed/not-found after stop completes. FD numbers are discovered by target path, never fixed. A terminal does not imply 6 |
| `runs[r].term` | Supervisor's validated `<run-dir>/terminal.v1`: absent → -1; command kind → actual code represented by 0/255; declared preparation/admission and source/preflight runner kinds → representative 75/78. A model representative does not permit dropping record kind or changing external 0–255 codes |
| `runs[r].receipt` | Finalizer's validated `<run-dir>/completion.v1`: absent → 0, `kind=complete` → 1, `kind=runner error=cleanup_failed` → 2; malformed data is the separately pinned artifact error, never a valid model state |
| `second_tree` | Input identity from both callers' actual checkout binding and canonical remote directory/lock target: 1 when equal, 2 when distinct. Read binding bytes and physical paths, not source-branch spelling |
| `source_revision[c]` | Checkout/source owner: read validated `<remote>/.overdrive-metal-source` `source_digest` plus the fixture's actual source bytes; map the two controlled input revisions to 0/1. The driver observes transferred state; successful action submission alone is not a source revision |
| `maintenance` | Exclusive owner's real barrier descriptor via the same fdinfo/path observation, its actual controlled maintenance-command process/output, and owner termination. 0 = pending acquisition, 1 = EX held while prior admitted units drain, 2 = maintenance command actually started, 3 = operation resolved by terminal/stop or admission timeout. Fixture command output is a real driven effect, not a driver-owned shadow state |
| `runs[r].ctl`, `.checked`, `.ever`, `.starts`, `.desc`, `.roots` | **Model-only** control-flow/history/resource-obligation ghosts. Do not project, add counters/accessors, or infer them from driver history. `.ever` remembers admission across main exit; `.desc` and `.roots` conservatively retain descendant/root obligations from admission, without claiming the physical roots already exist then. Native conformance separately inspects the actual unit process complement (`systemctl` ControlGroup plus `/sys/fs/cgroup/<ControlGroup>/cgroup.procs` and held VMM PIDs) after stop, and uses `lstat(2)` without following symlinks to verify absence of all three exact P5 roots on complete receipt. A cleanup-failed receipt must name its actual remaining/failing root |
| `observed`, `conf_step` | **Model-only** assertion history and replay stimulus. Never production projections. Compare each actual collector result to the modeled result at that action; do not supply these from a driver-owned result ledger |

The model-only replay envelope is `ConfStep = { tag: str, value: { actor: int, value: int } }`, consumed through the pinned quint-connect 0.1.2 path `Config::nondet = &["runner_conf::runner_protocol::conf_step"]` on `runner_conf`. It preserves the existing action names and actor/value meanings; the existing Idle action records an explicit Idle stimulus instead of repeating the preceding tag. This is a recording codec correction only: no projected state, production port or protocol action changes. [Exact codec limitation, stimulus mapping and validation boundary](design/recording-compatibility.md). Private Rust handler/fixture structure remains DISTILL-owned.

Model action/driving-port shapes are pinned at existing boundaries:

| Actions | Exact existing/proposed driving boundary |
|---|---|
| `Register`, `Barrier`, `Checkout` | Existing private durable service launch, original 15 positional values plus P2's exactly three values, with actual `ordinary`/`exclusive` class and named lock paths. Registration/control markers and kernel fdinfo determine completion of admission; fixtures may use the existing path-substitution test inputs |
| `Prepare`, `Sync`, `NoSyncRefusal`, `AbortPreparation` | Existing controller SSH/rsync/source-marker/native-preflight sequence under that service's checkout lock; `metal sync`, `metal run [--no-sync] -- …`, and the already-declared finite tagged ABORT. Prepare returns only after actual transfer and checks, not task submission |
| `Start`, `LoseController` | Exactly one `START` line on the existing control pipe; close that controller pipe/SSH route before or after START. Use the existing EOF/decoupling path; no public cancellation API |
| `AdmissionTimeout` | Existing configured admission deadline expires while a real holder prevents the requested lock; retained runner code/status must match P2 |
| `MainExit`, `SupervisorLost` | End the selected test-local fixture command through its existing process input, or terminate the real durable main process through its existing systemd owner. Before START, no body exists. A supervisor-loss trace validates the already-existing missing-terminal contract, not a new recovery guarantee |
| `StopOwner` | Existing blocking `systemctl stop overdrive-metal-run-<id>.service` / natural manager stop completion. Abstracts manager drain → P5 ExecStopPost → stopped unit. Inspect real processes/roots/receipt afterwards. Do not expose a production pause hook or add an intermediate cleanup protocol |
| `Collect` | Existing `metal collect <run-id>` or `lease-holder.sh --collect <id> <runs-root>`, with exact existing collector stdout/stderr/status, after its v2 wait-for-stop gate. Repeated collection is read-only |
| `Maintain` | Existing shell/provisioning or proposed `metal run --exclusive -- …` admission using the canonical EX barrier, then a controlled real maintenance fixture command. The same stop/liveness reader waits for already-admitted units; no cleanup daemon |

DISTILL shapes private fixture processes and conformance-driver internals. When a run observes an intermediate owner state, the existing fixture's selected command/descendant lifetime must make it observable; a forced inaccessible Rust state or a new production pause seam is forbidden. A native owner test must inspect actual process/root complements rather than projecting the resource-obligation ghosts. Finite assumptions are documented with their native/contract discharging items in the model README; implementation conformance against future v2 behavior remains a delivery obligation.

| Critical scenario | Named evaluator run |
|---|---|
| Independent worktrees run together, one completes without ending the other | `independent_worktrees` |
| Same checkout blocks its second owner and source sync | `same_checkout_excludes_sync` |
| Controller EOF before START yields no body | `controller_eof_before_start` |
| Controller loss after START retains actual durable result | `controller_loss_after_start` |
| Supervisor loss never fabricates a command result | `supervisor_loss_is_not_a_result` |
| Cleanup failure retains command terminal but refuses completion | `cleanup_failure_keeps_command` |
| Missing receipt refuses completion | `missing_completion_refuses` |
| Exclusive maintenance waits for unit stop after main exit | `maintenance_waits_for_stop` |
| No-sync mismatch refuses before START | `no_sync_mismatch_refuses` |
| Admission deadline expires before any body | `admission_timeout` |
| Native/source preflight aborts before START | `preflight_aborts_before_start` |
| Missing source-transfer rsync aborts with retained preparation status | `missing_rsync_aborts_before_start` |
| Repeat collection agrees | `repeat_collection_agrees` |

## Wave: DESIGN / [REF] Decisions and handoff

| DDD | Exact contract | Current verdict |
|---|---|---|
| DDD-1 / P1 | Canonical identity, fallback, override and migration | Proposed; user approval required |
| DDD-2 / P2 | Maintenance/shared admission and durable owner/wire changes | Proposed; user approval required |
| DDD-3 / P3 | Kernel namespace/resource/path boundary | Proposed; native construction held, user approval required |
| DDD-4 / P4 | Ordinary/exclusive entrypoints, metadata placement and per-attempt group lifecycle | Proposed; exact unavoidable compatibility choice must be surfaced |
| DDD-5 / P5 | Completion receipt and read-only collection gate | Proposed; actual timeout proof held, user approval required |

C4 context/container diagrams and the architecture brief contain proposal references only. Exact native findings and the closed placement set are retained with this feature. No unresolved specification ambiguity or contract-shaping validation task is handed downstream. Obtain explicit P1–P5 approval and independent design review before DISTILL/DELIVER. Implementation verification of the completed v2 runner, including actual two-worktree dogfood, remains the delivery obligation already required by DISCUSS. No roadmap is created here.

Focused proposed decisions: [ADR-0277](../../product/architecture/adr-0277-metal-checkout-identity-is-owned-by-the-local-git-worktree.md), [ADR-0278](../../product/architecture/adr-0278-metal-ordinary-runs-share-the-maintenance-barrier.md), [ADR-0279](../../product/architecture/adr-0279-metal-guest-resources-belong-to-the-durable-invocation.md), [ADR-0280](../../product/architecture/adr-0280-metal-nextest-execution-uses-explicit-test-placement.md), [ADR-0281](../../product/architecture/adr-0281-metal-run-completion-follows-owned-resource-cleanup.md). [C4 context and containers](../../product/architecture/c4-diagrams.md#metal-run-isolation-proposed).

## Wave: DESIGN / [REF] User approval — 2026-11-08

The user explicitly approved the concrete P1–P5 proposal in this conversation: **“Approve P1–P5”**. This accepts the exact contracts above, including worktree identity/fallback, shared maintenance admission with checkout exclusion, local-mode vsock namespaces, `metal run --exclusive` and ordinary nextest guest routing, and completion-gated collection. Independent technical review is APPROVED in `design/review.md`; earlier proposed/pending labels record the pre-approval chronology and are superseded by this approval. No accepted invariant is relaxed. Proceed with DEVOPS, DISTILL and DELIVER under this contract.

## Wave: DEVOPS / [REF] Status and authority

2026-11-08. **Platform readiness defined; ready for DISTILL.** This wave operationalises the accepted P1–P5 contract and its independent DESIGN review; it makes no implementation or production-readiness claim. Deployment is the checked-in Metal tooling consumed from each worktree. Reuse the configured `OVERDRIVE_METAL_TARGET`, existing native box, systemd invocation owner, current disposable guest groups, nextest, GitHub Actions and Lefthook. There is no new deployment platform, service, credential, orchestration layer or recurring job. The stopped service-feature DELIVER remains stopped.

## Wave: DEVOPS / [REF] Environment matrix

The machine inventory is [environments.yaml](environments.yaml). Its cases are fixture states on existing environments, not new runner settings or a Cartesian product. Use the actual Metal target from existing configuration; do not store its address or credentials in feature artifacts. Native construction evidence already held by DESIGN remains evidence for its recorded source/substrate, not for future v2 implementation.

| Case / existing environment | Preconditions and decisive dimension | Required observation / applicability |
|---|---|---|
| E1 / Metal | Distinct linked worktrees; include same-branch/different-dirty-source and different branch names with equal sanitisation | Full bindings, remote checkouts, locks, source markers, artifacts and invocation logs remain disjoint. Branch switch/detached linked worktree retains identity. Ordinary-clone branch fallback, detached-clone refusal and invalid/foreign override cases exercise P1 without multiplying guest runs |
| E2 / Metal | One bound checkout with a running command; competing run, sync or pull and controller loss | Same checkout stays excluded through the command; another checkout proceeds. Observe the durable owner's actual lock/process state, not a controller-local lock or action ledger |
| E3 / Metal and its disposable guests | Prepared current dispatcher, selected ELF/helpers, VMM/kernel/rootfs/module inputs; contrast individually missing required input, missing dispatcher and missing transfer rsync | No wrapper build or host fallback. Preserve the approved `lane_not_prepared`, `preflight_failed` and `runner_preparation_failed prerequisite=rsync` distinctions and 78/75 owner mappings. Host-pure bodies need no dispatcher |
| E4 / Metal | Ordinary runs, then an exclusive maintenance request; contrast waiting unadmitted runs and admitted units still stopping | SH ordinary owners coexist; EX maintenance starts only after admitted units finish stop/cleanup. One admission budget; maintenance/checkout timeouts remain distinct. `metal shell`, provisioning and `metal run --exclusive` retain the canonical EX barrier |
| E5 / Metal retained records | Existing v1 record and new v2 record; command terminal, stopped unit and complete/failed/missing/malformed completion states | Legacy v1 collection needs no receipt. v2 collection waits for owner stop and validates P5; repeat collection is read-only. Preserve missing-terminal 70 and artifact/cleanup 74 precedence, actual command 0–255, and nextest body-255/aggregate-100 distinction |
| E6 / Metal source | Matching and mismatched `--no-sync` binding/source marker, plus cold default identity and unbound legacy override | Mismatch refuses with `source_mismatch`/78 before START. Fresh default requires sync; legacy source/build caches and run records remain intact. Pull uses the bound checkout lock without requiring the current local dirty digest to equal the captured digest |
| E7 / actual paired Metal dogfood | `ouagadougou` and `san-antonio` both use the updated runner, each prepared from its actual source; capacity for two selected ordinary runs | Existing explicit guest-suite journey overlaps with distinct run IDs/source/logs; one supported failure/controller-loss case and repeat collection preserve exact results. Reuse the same capture for K1–K3; same-branch identity may use a controlled fixture rather than another full suite |
| E8 / physical Metal kernel complement | Before/after and surviving-run observations around E7 and supported terminal cleanup | Zero forbidden module/nftables/BPF/route/test-cgroup deltas; completed run has no remaining VMM descendants/owned ephemeral roots. Separate the finite unit/cgroup/process/KVM/vhost/socket/anonymous-namespace/image-root allow-set from test-kernel state; surviving invocation remains intact |
| E9 / existing native and guest contexts | Physical x86_64/KVM launcher, existing group/cell selection and exact source artifacts | Direct-native launch/confinement credit stays physical. Guest kernel effects stay in their guest. Existing `nested-boundary`, STEP 14 and VM-cohort cells retain only their named correctness credit. Each direct nextest kernel attempt/retry is fresh; explicit suite batching stays intact |
| Existing Lima `overdrive` / GitHub Actions `ubuntu-latest` | Current template/workflows, ordinary unit/Sim/build/doc/lint checks; non-Metal wrapper context | Preserve existing execution and CI reports. Lima and hosted CI do not qualify direct-native VMM evidence or the physical-host concurrency/cleanup claim |

Only the pinned Linux 7.0 family is credited for real-kernel assertions; DESIGN observed `7.0.0-38-generic`. Record actual kernel/tool/source/dirty state for later captures. Existing capacity is finite: two standard 4,096 MiB groups need at least 8,192 MiB guest RAM plus the current reserves, launchers and builds. Keep the existing per-nextest `host-kernel-shared` max-one guest capacity. No density campaign, speedup ratio, additional OS/version matrix or new host is required.

## Wave: DEVOPS / [REF] CI/CD pipeline outline

| Stage / existing trigger | Reused gate and this feature's boundary |
|---|---|
| Local pre-commit / staged Rust paths | Existing Lefthook formatting, scoped Clippy/doctests and affected nextest. `infra/hooks/linux-run.sh` selects configured Metal or existing Lima. Metal preparation must obey the accepted ordinary boundary: build inputs without an implicit physical-host `kmod-load`; selected kernel bodies use their guest placement. Required hook fallout implements this accepted invariant, not new host authority |
| Local pre-push | Existing conformance lint and workspace nextest; preserve filters, property budgets and Linux execution. Current hook `kmod-load` preparation must not become an ordinary host mutation or an automatic exclusive fallback |
| PR to `main`, push to `main`, manual `workflow_dispatch` / existing `ci.yml` | Keep current jobs and statuses: fmt/Clippy, nextest/doctests, integration, DST/lints, BPF build/unit and existing verifier/performance gates. Outside ordinary Metal, the checked-in wrapper passes the original execution chain unchanged. No new CI framework, secret, native-host job, workflow cancellation policy or kernel matrix |
| Existing `quint` CI job / same triggers | Existing typecheck, `check --ci`, `verify-evidence` discover the permanent `metal-run-isolation` manifest; retain its 18 expected outcomes and trace/source-hash provenance. No additional model family or redundant native rerun |
| Feature delivery / existing Metal commands | DISTILL's bounded runner/placement/source/collector checks and the one actual two-worktree capture prove the new composition. Existing native construction proofs are premises, not a substitute for implemented-path acceptance. Keep crate tests and external expectation evidence at their established independent boundaries |
| Existing scheduled workflows | No added schedule or feature-specific benchmark. The explicit no-mutation instruction below governs this feature session; existing workflow definitions are not changed by this documentation wave |

These are required later-wave verification placements, not checks executed by DEVOPS. Existing pure shell/record fixtures may test their own substituted paths; host-adapter effects require real native observations. DISTILL keeps each test's Contract Shape declaration and ensures intended RED failures reach the domain assertion rather than missing placement or lane preparation.

## Wave: DEVOPS / [REF] Monitoring contracts

DISCUSS owns the K1–K3 targets/baselines above. The feature implementer retains raw observations; an independent reviewer evaluates them. The following uses only approved records, markers and existing external tools. No new event schema, production counter, accessor or log field is introduced.

| KPI / exact target | Collection and attribution | Maintainer view / failure threshold |
|---|---|---|
| K1 — 2/2 paired runs overlap; 0 box-wide ordinary execution-lease waits | Correlate printed `OVERDRIVE_METAL_RUN_ID`, validated `metadata.v2` execution class/checkout binding, START/completion output and real selected-command process intervals. Observe canonical barrier descriptor paths and SH/EX mode via `/proc/<MainPID>/fd*` plus `systemctl show`; distinguish expected own-checkout/maintenance waits from forbidden ordinary global exclusion | One retained paired-capture table with each run's source and actual execution interval. No overlap or any unrelated ordinary EX execution barrier fails K1; bounded capacity is a precondition, not a speed benchmark |
| K2 — 100% of contrasting runs have correct source/log/status; 2/2 repeat collections agree | Capture the source marker/actual contrasting bytes while each checkout is owned; compare each run's retained metadata/source digest, `checkout.v1`, original command and exact log/terminal with `metal collect <run-id>` output/status. Include post-START controller loss and one actual nonzero result; retain runner-vs-command kind even when numeric codes coincide. For v2 inspect owner stop and valid `completion.v1` before crediting collection | Same capture has source/log/result and repeated-collection columns. Any source/evidence mixing, changed repeated result, fabricated command result or runner failure presented as success fails K2. Existing malformed/supervisor-loss outcomes are diagnostic failures, not command results |
| K3 — 0 forbidden physical test-kernel deltas; 0 completed-run ephemeral resources; 0 cleanup changes to survivor | Bounded read-only snapshots with existing `/proc`, `systemctl`, `ss`, `nft`, `bpftool`, `ip` and filesystem tools: compare physical initial-namespace module/nftables/BPF/routes/test-cgroup state and vsock modes; identify permitted run-owned launcher namespaces, sockets, device/process/cgroup and image roots separately. After actual unit stop inspect process/cgroup complement and `lstat` absence of exactly `/srv/vm/metal/<id>`, `/run/ovd-gk/<id>` and `<recorded-remote-dir>/target/metal-run-tmp/<id>`. Observe the other invocation's same resource set and continuing guest/control output | Same capture records before/after and surviving-owner observations. Any forbidden delta, completed-run residue or attributable survivor change fails K3; unavailable observations cannot be credited as zero. Cleanup-failed receipt remains typed failure 74 with actual command terminal retained |

Failure thresholds block the feature's acceptance claim; the maintainer sees existing stderr markers and retained evidence. No new paging, alerting backend or persistent dashboard is needed for this one delivery capture. `docs/product/kpi-contracts.yaml` explicitly belongs only to `docs-platform`; adding these tooling KPIs there would corrupt that SSOT. This section is the metal feature's instrumentation contract and links the existing DISCUSS KPI definitions without changing that website file.

## Wave: DEVOPS / [REF] Deployment strategy

Rollback first: finish or explicitly stop affected invocation units through their existing systemd owner and retain their actual terminal/completion/logs before changing tooling; preserve every checkout binding, old cache and run record. A tooling revision with the v2 collector can still collect v1; keep that reader available for retained v2 records rather than pretending an old reader understands them. Do not downgrade an in-flight v2 owner/finalizer or automatically remove its records/roots. After owners finish, selecting a prior tooling revision restores its old exclusive behavior for subsequent invocations, with no cache adoption or source deletion. Rollout is the normal reviewed source change, then prepared updated tooling in each participating worktree and the bounded paired capture before claiming the outcome. Older branches remain exclusive as P2 states. This is checked-in developer tooling, with no service publication, fleet deployment, database migration or canary percentage. Rollback/rollout implementation validation remains DELIVER work; no deployment was executed here.

## Wave: DEVOPS / [REF] Mutation testing strategy

**Disabled for `metal-run-isolation` in this session**, honoring the user's earlier explicit **“NO mutation testing”** instruction. Do not execute per-step or final feature mutation testing or edit mutation exclusions to compensate. This feature decision overrides installed skill defaults; it does not rewrite project-wide `CLAUDE.md`, global configuration or existing CI schedules. Correctness evidence remains the accepted formal checks, meaningful runner/placement/owner tests, bounded native observations and independent review. P4's existing mutation-command routing compatibility is still part of its API contract; this instruction does not authorize executing it.

## Wave: DEVOPS / [REF] Observability stack

| Signal | Existing instrument / boundary |
|---|---|
| Logs and exact results | Durable `/var/lib/overdrive-metal/runs/<id>/` records and combined log; existing run/runner/test error markers and `metal collect`. Validate v1/v2 filename/format pairs and preserve exact terminal kind/status |
| Outcome measurements | The K1–K3 bounded capture above, annotated with source/substrate and evaluated independently. Actual command intervals and external resource snapshots supply measurements; there is no new metrics service or performance percentile claim |
| Execution/ownership trace | Existing systemd unit properties, kernel descriptor/namespace/process observations and run/source identity. Model ghosts are never production telemetry; no distributed tracing agent or runtime instrumentation is added |
| Health and diagnostics | Existing fail-closed native/source/owner preflight and typed wrapper/collector errors. Admission and completion are tooling states, with no Overdrive product readiness, health or lifecycle meaning |

## Wave: DEVOPS / [REF] Branching strategy

Retain the existing feature-branch → reviewed PR → `main` workflow. `.github/workflows/ci.yml` already triggers on PRs to `main`, pushes to `main` and manual dispatch. This change adds no release branch, promotion environment or branch-protection setting. Worktree identity is independent of a linked worktree's branch; two branches with the same sanitised name and two worktrees on one branch must remain separate even when CI uses the same branch workflow.

## Wave: DEVOPS / [REF] Coexistence matrix

| Existing tool / owner | Must continue to work alongside the change |
|---|---|
| Git worktrees, standalone remote Git copy, SSH/rsync | Preserve full identity binding, cold migration, existing override validation, source marker and same-checkout lock. Ordinary operations require installed rsync and never install host packages |
| `metal sync/run/pull/collect/shell`, provisioning | Preserve exact source/status/retrieval boundaries, read-only repeated collection and explicit maintenance exclusivity. Existing `--no-sync`/`--no-sudo` do not change identity or ordinary/exclusive authority |
| Nextest profiles, selection, retry/reporting, configured target runner | Preserve the mandatory ordinary adapter and selected ELF/argv/helpers/property/attempt context. Host-pure needs no guest; unknown placement refuses. Outside ordinary Metal, exec the original chain unchanged |
| Existing guest suite and fixtures | Retain eight named groups and existing operator/owner/nested cells, explicit batching, actual production fork/confinement and current CID bands. Run-owned short paths and finalization must leave another invocation intact |
| Lefthook and agent command guards | Keep actual canonical Linux routing and existing checks; adapt only the host-load preparation/messages that conflict with accepted ordinary guest execution. No automatic elevation of arbitrary commands into exclusive maintenance |
| Existing Rust CI, Lima `overdrive`, website workflows | Preserve current jobs, triggers and non-Metal behavior; no new platform or hook framework and no website KPI/config changes |
| Concurrent/legacy invocations | Two updated ordinary callers are the credited concurrency case. Old exclusive callers stay exclusive; retained v1 collection remains valid. No automatic legacy-directory adoption, pruning or recovery daemon |

## Wave: DEVOPS / [REF] Pre-requisites and DISTILL handoff

P1–P5 approval is recorded above and independent DESIGN review iteration 2 is APPROVED. No material design decision is reopened. The exact driving ports, record/control formats, semantic error set and native-versus-guest applicability in DESIGN remain authoritative; this wave adds no API, flag, variable, error or lifecycle state. [environments.yaml](environments.yaml) supplies environmental realism for AC-1..AC-7. The [closed 4,640-identity/94-binary inventory](design/placement-inventory.md), source/native proofs and [18-check protocol evidence](../../../specs/quint/metal-run-isolation/evidence/summary.md) are held construction inputs. Future new tests need explicit placement and Contract Shape declarations, including `/// CONTRACT_SHAPE: pure-function.` for source-local pure Rust properties.

DISTILL should shape executable coverage over E1–E9 at their actual boundaries, reusing observations across K1–K3 rather than repeating guest suites for every identity/error case. It must cover prepared and absent prerequisites, source/identity refusal, lock ownership, exact retained/collected failures and results, nextest routing/retry freshness and supported terminal cleanup. Per-condition assertions must use DESIGN's exact typed mappings; a generic carrier/string or wrapper refusal is not an approved substitute. External evidence drives the actual developer commands and observes the host/guest/resource complement; Rust tests stay in-process and cannot become expectation runners. No acceptance test, expectation capture, native rerun, roadmap, implementation or commit was produced in DEVOPS.

Density was resolved through the installed `scripts.shared.density_config.resolve_density`: `lean`, `ask-intelligent`, `explicit_override`; only required reference sections are emitted. The installed/repository feature-layout and feature-delta validators and the shared density telemetry helper were not found. No substitute tool or hand-written audit event was added. Available mechanical documentation gates passed: YAML parsing, ten required DEVOPS sections, four existing environments, E1–E9 coverage of AC-1..AC-7/K1–K3, DEVOPS link targets and whitespace checks. This does not claim runtime verification. No upstream assumption changed, and no deployment-topology/KPI SSOT extension outside this feature is required. Optional per-wave platform review is skipped: no novel target, CI framework, observability rewrite or secret/RBAC change is introduced; the consolidated DISTILL review remains required.

## Wave: DISTILL / [REF] Scope, reconciliation and status

2026-11-08. `[lang-mode] Rust` with existing Bash tooling adapters. `[policy-mode] inherit`: existing project testing/infrastructure conventions, refined here for the already-approved Metal driving ports. Strategy A retains real runner/guest ownership. P1–P5 approval and design-review iteration 2 supersede historical proposal/status labels; DISCUSS, DESIGN and DEVOPS contain **zero unresolved decision contradictions**. E1–E9 and K1–K3 define dimensions and observations, not a Cartesian boot matrix. The stopped service feature and all its dirty work remain untouched.

Acceptance authoring is **ready for consolidated independent review**, with [15/15 mechanical checks](distill/completeness.md) and no remaining authoring gap delegated to a crafter. All bodies remain pre-authored pending activation where implementation is missing; this is not an implementation-pass or dogfood claim. The required recording codec gap was returned to DESIGN, corrected without protocol changes and independently approved. No new production API/CLI/wire/error choice, production implementation, roadmap or commit is introduced here.

## Wave: DISTILL / [REF] Scenario bindings and tags

[Readable GIVEN/WHEN/THEN specification](distill/test-scenarios.md); no `.feature` file or Gherkin consumer. Rust bodies are under `xtask/tests/acceptance/metal_run_isolation/`, wired by the existing `xtask::acceptance` entrypoint. All new Rust cases carry reasoned pending-DELIVER markers. Exact test names below are relative to `acceptance::metal_run_isolation::`.

| Scenario / tags | Executable binding | AC / environment |
|---|---|---|
| Verify independent changes together; `walking_skeleton driving_port real_io kpi` | `infra/metal/tests/worktree_journey.sh` | AC-1/2/4/5/6; E2/E7/E9; K1–K3 |
| Equal branch source/artifact complement; `driving_port` | `identity::same_branch_worktrees_keep_source_artifacts_and_evidence_disjoint` | AC-2; E1/E6 |
| Branch switch/detachment preserves linked owner | `identity::linked_identity_survives_branch_switch_and_detachment` | AC-2; E1 |
| Clone fallback/sanitised spellings/cold migration | `identity::attached_clone_fallback_and_sanitised_spellings_never_adopt_old_state` | AC-2; E1/E6 |
| Invalid discovery/override/binding; `sad` | `identity::sad_invalid_identity_and_override_never_contact_a_mutating_remote`, `identity::sad_override_conflicts_preserve_the_full_remote_complement` | AC-2; E1 |
| Pull after dirty edit / repeated sync | `identity::pull_uses_binding_after_local_source_changes_and_repeated_sync_preserves_target` | AC-2/3; E6 |
| Transfer prerequisite refusal; `sad infrastructure_failure` | `identity::sad_missing_rsync_is_typed_without_installing_packages`; full `controller::sad_registered_missing_rsync_keeps_its_preparation_error_and_no_sync_needs_no_rsync` producer sequence | AC-6; E3 |
| Actual no-sync producer decision; `sad` | `controller::sad_actual_no_sync_mismatch_aborts_the_registered_owner_before_start` | AC-2/3/6; E6 |
| Own-checkout/maintenance timeout; `sad` | `admission::sad_own_checkout_and_maintenance_deadlines_have_distinct_causes`, `admission::sad_sync_and_pull_wait_on_the_running_checkout_without_source_mutation` | AC-1/3/7; E2/E4 |
| Before-START control refusal; `sad` | `admission::sad_prestart_controls_retain_the_approved_error_set_without_a_body` | AC-3/6; E2/E5 |
| After-START controller loss / one-shot start | `admission::independent_ordinary_owners_overlap_after_controller_loss`, `admission::duplicate_start_never_replays_a_selected_command` | AC-1/3/6; E2/E5 |
| Explicit maintenance waits for actual stopped owner | `infra/metal/tests/admission_journey.sh` | AC-1/3/5/7; E2/E4/E5 |
| Ordinary grammar/authority/config/artifact refusal; `sad` | `infra/metal/tests/placement_journey.sh`, `identity::sad_exclusive_is_only_a_run_mode_at_both_existing_entrypoints` | AC-4/7; E8 |
| Actual selected ELF/helper/environment/retry/status | Standalone `infra/metal/tests/fixtures/attempt/` driven by `placement_journey.sh`; `placement::host_pure_attempt_preserves_exact_arguments_environment_and_255_without_a_guest` | AC-4/6; E3/E8 |
| Non-Metal/exclusive coexistence | `placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain` | AC-4/7; E8 |
| Closed placement/context/no dispatcher; `sad` | `placement::sad_invalid_context_missing_identity_and_missing_guest_dispatcher_never_run_the_body`, `placement::sad_invalid_duplicate_and_ambiguous_placement_is_fail_closed`, `placement::runtime_placement_preserves_the_reviewed_partition_and_declares_new_acceptance_identities` | AC-4/7; E3/E8 |
| Anonymous local namespace / all operation refusals; `real_io sad` | Native context, parent permission, readonly child_mode, private namespace-capacity and actual verification-reader descriptor-exhaustion cases in `placement`; bounded `namespace_trace` fixture controls | AC-4/7; E9 |
| Exact native/nested/operator/owner applicability | Existing exact production tests selected by `placement_journey.sh`; retained native construction is prerequisite evidence | AC-4/7; E9 |
| All native prerequisite probe diagnostics; `sad infrastructure_failure` | `qualification::sad_each_existing_native_prerequisite_has_the_exact_probe_and_retained_cause` | AC-6/7; E3/E9 |
| Legacy/current repeat read-only collection | `collection::retained_v1_and_v2_commands_return_exact_status_on_read_only_repeat_collection` | AC-5/6; E5 |
| Actual owner-stop collection gate | `collection::collection_waits_for_owner_stop_even_when_terminal_and_receipt_exist`, native `admission_journey.sh` | AC-5/6; E5 |
| Scoped finalization and actual three-root failures | Both `finalization` cases: exact repeat cleanup/complement and actual immutable-file refusal at each declared root, then real collection/74 | AC-5/6; E5 |
| Malformed/cleanup/supervisor precedence; `sad` | `collection::sad_cleanup_and_artifact_failures_never_become_command_results`, `collection::sad_retained_records_enforce_closed_error_and_command_status_domains` | AC-5/6; E5 |
| Invalid/unknown retrieval; `sad` | `collection::sad_collection_without_a_run_preserves_absence` | AC-6; E5 |

The readable specification has 20 scenarios, 11 negative/failure scenarios (55%). Finite rows share fixtures rather than parameterising native boots. The three shell drivers are rerunnable developer-tool E2E tests and drive `cargo xtask metal`, never product expectations or the built Overdrive CLI. Their Python observer only reads actual host/owner facts; no model ghost is projected.

## Wave: DISTILL / [REF] Adapter coverage, placement and prerequisites

| Boundary | Test adapter / independent observation | Honest evidence limit |
|---|---|---|
| Bootstrap Git/SSH/rsync | Real Git clones/worktrees, real local rsync, existing-style fake remote command transport, exact source/build/evidence snapshots | SSH partition fixture grants no remote/native/kernel evidence |
| Durable admission/control | Actual `lease-holder.sh --durable` eighteen-value entry, real Linux flocks, fdinfo, processes/control pipe and retained owner/terminal records | Private entry tests do not imitate systemd cleanup; native external driver checks actual unit stop and exclusive body start |
| Retained collector | Actual `--collect` entry over valid/malformed v1/v2 preconditions, output/status plus complete byte snapshots | Static retained fixtures prove reader semantics, not a producer's fault cause |
| Finalizer | Actual declared `--finalize` entry, fresh validated run ID and exactly its three real ephemeral roots | Repeated exact cleanup/complements plus pre-authored real immutable-file failures at all three roots; pending implementation execution |
| Native qualification | Existing external probe input substitutions, exact typed markers/statuses | Negative adapter partitions; fake healthy probes earn no native boot credit |
| Nextest placement/dispatch | Actual wrapper and runtime policy, actual attempted executable argv/env/status; existing suite plus standalone Cargo-reported ELF/helper under external nextest | Missing production wrapper remains pending; no fake dispatcher returns fabricated guest results |
| Guest/native lifecycle | Existing production VMM/GuestKit groups, native and specifically named nested/operator/owner bodies; read-only real process/namespace/CID/root/kernel observations | Pending external execution; prior DESIGN construction evidence is retained without repeating its campaign |

[New exact placement declarations](distill/test-placement-additions.json) extend the approved inventory under its existing DISTILL extension semantics: 36 Rust acceptance identities, 15 owning-crate conformance identities and three standalone guest fixture identities. Owned namespace fixtures are native-launcher; the actual profile canary and standalone bodies use `guest-group/core`; remaining tooling/file/process cases are host-pure. The two transitioned original qualification identities retain their existing partition. DELIVER merges these explicit descriptors into runtime policy; no absence-means-host rule or brittle total count is permitted. The standalone fixture reuses the mandatory wrapper and existing default nextest profile with a bounded test-input timeout; it creates no recurring lane or product CLI.

Prerequisites: accepted P1–P5 / independent DESIGN approval, existing 18-check Quint evidence and native construction evidence, E1–E9 environments, provisioned rsync/native substrate, updated callers and current prepared dispatcher/selected guest artifacts for native E2E. Reuse the source/kernel/resource snapshots across K1–K3. [Validation](distill/checks.md) establishes compiled tests and semantic P1 RED; no v2 concurrency, completed dogfood or other unexecuted behavior is claimed. The [self-audit](distill/completeness.md) records every prior gap's closure, exact owner/failure paths, conformance projection and validation limits. Independent review and eventual implementation checks remain required.


## Wave: DISTILL / [REF] Owner model, composed failures and activation handoff

`xtask/src/metal_run_isolation_conformance.rs`, test-only wired from `lib.rs`, references the sole permanent `runner_conf` model and implements every recorded action. The thirteen `protocol_<named-run>` cases bind one-to-one to DESIGN's thirteen critical runs; `protocol_randomized_owner_traces` adds four bounded 24-step traces with reproducible seed output. `protocol_illegal_controls_preserve_each_actual_owner_state_boundary` covers illegal inputs at each addressable state. All retained model ghosts remain excluded from owner projection. Current Collect's model assertion is compared with the actual returned collector process result once, not a driver-owned history ledger.

The real protocol composition owns transient units, the captured actual durable/finalizer code, discovered real flock descriptors and validated actual records/source. External flock/finalizer-process fixtures only control operation timing/faults; no production pause hook or accessor is added. Full-bootstrap `controller` tests independently prove the production controller selects source_mismatch/78 or runner_preparation_failed/75 before START, finalizes, and collects the same result. A separate two-lock test preserves the real single deadline. Namespace setup failures use real scoped capability/readonly/capacity/descriptor limits; the verification fixture never replaces a syscall result. Guest execution failure resets only a positively identified disposable guest and retains the original peer/VMM error chain. Cleanup failures are real immutable-file deletion refusals with restoration scoped to their exact owned roots.

The pre-authored transition bodies in `legacy_contract.rs` replace the two obsolete qualification premises in `main.rs` under their existing test identities. They preserve maintenance/checkout exclusion, holder diagnostics, cancellation/reacquisition and explicit selected-artifact reinjection without sudo environment preservation. Their authority is now the approved `--exclusive` flag; the unchanged original native-prerequisite oracle remains. Crafters remove pending markers and implement the accepted contract; they do not rewrite test bodies. Existing `host-kernel-shared` capacity is assigned to the new profile canary, and the standalone input fixture reuses that same capacity owner. No new lane or recurring gate is invented.

Activation guidance for roadmap input: P1 identity/transfer cases first; real P2/P5 owner-protocol and full-controller refusal cases when those exact interfaces are implemented; P3 context/cleanup cases with scoped root preparation; P4 placement/helper/retry/profile cases after the prepared dispatcher and runtime inventory exist. The single external walking-skeleton capture still validates the whole outcome. This is dependency guidance, not a new roadmap or permission to execute a pending roadmap.

The scope expands only into necessary test wiring/configuration: existing xtask test-only modules, existing-workspace dev dependencies/lockfile entries, nextest capacity assignment and new test fixtures. No executable production branch, public method/type/variant, product state or CLI beyond approved P1–P5 is introduced. Every new identity and existing transition is listed in the machine placement handoff. Capture must use empty directories outside the input worktrees, preserving source identity and first-failure diagnostics. Actual default/ci guest canaries, boot-guarded timeout/reset, selected ELF/helper/retry data, named native/nested/operator/owner selections and read-only kernel/process/root complements remain pending DELIVER E2E execution.


## Wave: DISTILL / [REF] Explicit verification waiver

The user directed **“STOP RUNNING TESTS THAT REQUIRE THE METAL LEASE”** and waived additional pre-implementation Metal executions for this handoff. No further lease-acquiring check, direct-SSH bypass or alternate hidden Metal execution is required or permitted here. [checks.md](distill/checks.md) records the existing Linux Clippy pass and seeded semantic RED, the permission-fixture failure and local adjustment, and every remaining check as NOT RUN. The 15/15 authoring audit does not convert these unexecuted checks into runtime success. No own active Metal verification controller remained at the stop check; no other workspace owner was touched. Consolidated review and the subsequent authorised implementation workflow can proceed from these complete authored bodies with this truthful evidence boundary.

## Wave: ROADMAP / [REF] Delivery sequence and activation

[The CLI-scaffolded roadmap](deliver/roadmap.json) contains two sequential implementation steps and remains **PENDING independent roadmap review**. This section supplies roadmap-only execution/activation notes; it changes no approved P1–P5 interface, invariant, error mapping, placement or environment. The four consolidated [requirements](distill/review-requirements.md), [architecture](distill/review-architecture.md), [platform](distill/review-platform.md) and [acceptance](distill/review-acceptance.md) reviews are APPROVED. Their authored-coverage approval does not claim runtime success.

### Dependency cut and quantitative basis

1. **Bind each checkout's source and retained artifacts** implements P1 through existing sync/pull/run consumers and activates seven identity/transfer cases. Existing exclusive command execution remains in force. Source attribution can improve independently without admitting concurrent ordinary execution.
2. **Run ordinary verification concurrently with owned guest cleanup** depends on the first step and implements the remaining P2–P5 composition together. Shared ordinary admission must not become executable before its exact source ownership, finite command grammar, mandatory guest/native placement, local namespace, dispatcher preparation, invocation roots and descendant-stop/finalizer guarantees are complete. No transitional flag, temporary host fallback or intermediate lease-removal release is permitted. The retained `--exclusive` administrative boundary and same-checkout exclusion remain required.

These are internal implementation cuts within the **one release slice**, not two independently accepted feature releases. Separating lease removal from routing/cleanup would violate AC-4/AC-5. The closed P4 inventory and its explicit DISTILL additions are reused, without a scheduler, allocator, new model, pool or recovery mechanism. Configuration-only lease removal leaves the observed singleton/source/resource hazards unresolved; a wrapper-only change leaves ordinary arbitrary-command authority and durable completion wrong. DESIGN's existing rejected alternatives remain authoritative; no architecture choice is reopened here.

The measured/observed planning baseline is one admitted box-wide command, the target is two overlapping ordinary executions, and the reviewed initial placement inventory has 4,640 identities across 94 binaries. Capacity remains two 4,096 MiB groups plus existing reserves. There is no speedup or benchmark claim. The two-step estimate is **2 + 8 crafter hours**, a planning revision of DISCUSS's expressly provisional six hours after the complete owner/placement/error corpus became available, never an effort ceiling. Two steps across at least sixteen production/tooling/configuration files is below the decomposition-ratio limit. Native boot cost is bounded by the already-authored selections and reused K1–K3 capture.

### Common crafter and reviewer gates

- Read the exact approved DESIGN P1–P5, typed failures, lifecycle/model action tables, DEVOPS and DISTILL sections above; implement those interface contracts without inventing or restating a different public/cross-boundary shape. Missing semantic error set/representation is **SPECIFICATION_AMBIGUITY**: pause only the affected implementation and return it to DESIGN. Private implementation/test-support details are not DESIGN blockers. A missing acceptance body/oracle belongs to the acceptance author.
- Crafters remove only the assigned **pending DELIVER** attributes from the complete pre-authored cases below. They do not author, complete, replace or weaken acceptance bodies, stimuli, universes or assertions. The deliberately ignored external bodies/canary retain their ignores. Existing live namespace fixture controls retain their bodies and placement.
- **TEST PARADIGM:** Rust unit properties use `proptest` and the repository's strict state-delta discipline over the declared universe, including its unchanged complement; no Python/Hypothesis translation or new assertion framework. Real-I/O integration and E2E remain finite examples at their legitimate tooling/owner boundaries. Every new/transitioned live case retains its Contract Shape declaration; a source-local pure Rust property requires exactly `/// CONTRACT_SHAPE: pure-function.`. Any missing authored coverage returns to DISTILL.
- A fresh isolated crafter executes each step's actual RED → GREEN → COMMIT, followed by its fresh independent reviewer. Remediation returns to that step's original crafter and reviewer until APPROVED. No next step begins before its predecessor's native Markdown `deliver/review-{step-id}.md` exists and approves it. Only the executing crafter writes its actual DES events; the orchestrator initializes the log with `des-init-log`, never hand-written events. Session routing is GPT 6.1 Sol for roadmap/upstream roles and GPT 6 Luna at maximum thinking for DELIVER crafters/reviewers.
- Reviewer evidence must map each designed semantic failure to the approved implemented marker, payload, status, retention and precedence. Green tests cannot approve a divergent interface. Suspected additional production defects require the repository's reachable-path proof and failing reproduction; this roadmap does not authorize adjacent hardening or architecture growth.
- File lists guide the necessary bounded fallout; they are not restrictive allowlists. Document any required additional consumer/configuration/compiler edit against the acceptance criterion. Preserve all pre-existing dirty work, including stopped service-feature edits and existing DISTILL wiring in shared files. Use focused owned-path commits with the existing Git author and exactly `Co-Authored-By: Codex <codex@openai.com>`. No mutation testing, exclusion changes or stopped-feature delivery.

### Source ownership activation

All names here are in binary `xtask::acceptance`, file `xtask/tests/acceptance/metal_run_isolation/identity.rs`, under `acceptance::metal_run_isolation::identity::`. The first roadmap step removes exactly these seven current pending markers:

- `same_branch_worktrees_keep_source_artifacts_and_evidence_disjoint` — `pending DELIVER: metal-run-isolation canonical checkout identity`.
- `linked_identity_survives_branch_switch_and_detachment` — same canonical-checkout-identity marker.
- `attached_clone_fallback_and_sanitised_spellings_never_adopt_old_state` — same canonical-checkout-identity marker.
- `sad_invalid_identity_and_override_never_contact_a_mutating_remote` — `pending DELIVER: metal-run-isolation identity refusals`.
- `sad_override_conflicts_preserve_the_full_remote_complement` — `pending DELIVER: metal-run-isolation canonical binding`.
- `pull_uses_binding_after_local_source_changes_and_repeated_sync_preserves_target` — `pending DELIVER: metal-run-isolation source fidelity`.
- `sad_missing_rsync_is_typed_without_installing_packages` — `pending DELIVER: metal-run-isolation source preparation`.

The eighth identity case, `sad_exclusive_is_only_a_run_mode_at_both_existing_entrypoints`, retains its `pending DELIVER: metal-run-isolation explicit maintenance mode` marker until the integrated step implements that exact approved CLI boundary.

### Integrated owner, placement and completion activation

The second roadmap step removes all remaining **pending DELIVER: metal-run-isolation …** markers in the following files, preserving their complete bodies. Acceptance names below are relative to `acceptance::metal_run_isolation::` in `xtask::acceptance`.

| File under `xtask/tests/acceptance/metal_run_isolation/` | Exact case names | Existing pending-marker subjects |
|---|---|---|
| `identity.rs` | `identity::sad_exclusive_is_only_a_run_mode_at_both_existing_entrypoints` | explicit maintenance mode |
| `admission.rs` | `admission::independent_ordinary_owners_overlap_after_controller_loss`; `admission::sad_own_checkout_and_maintenance_deadlines_have_distinct_causes`; `admission::sad_prestart_controls_retain_the_approved_error_set_without_a_body`; `admission::duplicate_start_never_replays_a_selected_command`; `admission::sad_sync_and_pull_wait_on_the_running_checkout_without_source_mutation`; `admission::sad_waiting_for_both_locks_cannot_restart_the_existing_admission_budget` | durable shared admission; checkout exclusion; preparation abort protocol; one-shot durable start; source exclusion; single admission deadline |
| `collection.rs` | `collection::retained_v1_and_v2_commands_return_exact_status_on_read_only_repeat_collection`; `collection::collection_waits_for_owner_stop_even_when_terminal_and_receipt_exist`; `collection::sad_cleanup_and_artifact_failures_never_become_command_results`; `collection::sad_retained_records_enforce_closed_error_and_command_status_domains`; `collection::sad_collection_without_a_run_preserves_absence` | completion-gated collection; unit-stop collection gate; completion error taxonomy; retained protocol validation; retrieval refusal |
| `controller.rs` | `controller::sad_actual_no_sync_mismatch_aborts_the_registered_owner_before_start`; `controller::sad_registered_missing_rsync_keeps_its_preparation_error_and_no_sync_needs_no_rsync` | composed no-sync refusal; composed transfer prerequisite |
| `finalization.rs` | `finalization::finalization_removes_exact_owned_roots_and_repeating_it_preserves_the_complement`; `finalization::sad_each_owned_root_removal_failure_is_produced_and_collected_with_its_exact_root` | scoped finalization; cleanup failure producers |
| `placement.rs` | `placement::host_pure_attempt_preserves_exact_arguments_environment_and_255_without_a_guest`; `placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain`; `placement::sad_invalid_context_missing_identity_and_missing_guest_dispatcher_never_run_the_body`; `placement::sad_invalid_duplicate_and_ambiguous_placement_is_fail_closed`; `placement::native_launcher_gets_a_local_vsock_namespace_without_changing_the_initial_mode`; `placement::sad_namespace_permission_denial_never_falls_back_to_the_host`; `placement::runtime_placement_preserves_the_reviewed_partition_and_declares_new_acceptance_identities`; `placement::sad_each_later_namespace_operation_keeps_its_exact_step_and_errno`; `placement::sad_actual_verification_read_failure_preserves_its_step_and_errno` | mandatory nextest placement; non-Metal coexistence; closed placement refusal; placement schema validation; native launcher resource context; namespace refusal; complete test placement; namespace operation failures; namespace verification failure |
| `qualification.rs` | `qualification::sad_each_existing_native_prerequisite_has_the_exact_probe_and_retained_cause` | typed native qualification |

All fifteen cases in `xtask/src/metal_run_isolation_conformance.rs`, binary `xtask`, activate in this integrated step under `metal_run_isolation_conformance::`:

- `protocol_independent_worktrees`;
- `protocol_same_checkout_excludes_sync`;
- `protocol_controller_eof_before_start`;
- `protocol_controller_loss_after_start`;
- `protocol_supervisor_loss_is_not_a_result`;
- `protocol_cleanup_failure_keeps_command`;
- `protocol_missing_completion_refuses`;
- `protocol_maintenance_waits_for_stop`;
- `protocol_no_sync_mismatch_refuses`;
- `protocol_admission_timeout`;
- `protocol_preflight_aborts_before_start`;
- `protocol_missing_rsync_aborts_before_start`;
- `protocol_repeat_collection_agrees`;
- `protocol_randomized_owner_traces`;
- `protocol_illegal_controls_preserve_each_actual_owner_state_boundary`.

The first thirteen have the existing `real-owner protocol conformance` pending subject, the randomized case has `seeded real-owner protocol conformance`, and the last has `controls across actual owner states`. They reuse the sole permanent model and its existing thirteen named runs/four bounded randomized traces. No model rewrite, synthetic projection, new check or replay-result ledger is authorized.

The same step removes the two `pending DELIVER: metal-run-isolation exclusive qualification transition` attributes in `xtask/src/main.rs`, binary `xtask::bin/xtask`, under `metal_qualification_tests::`:

- `metal_run_and_bootstrap_share_the_global_lease_and_sync_takes_only_the_tree_lock`;
- `metal_run_reinjects_guest_artifacts_without_sudo_environment_preservation`.

Their complete pre-authored delegates in `xtask/tests/acceptance/metal_run_isolation/legacy_contract.rs` remain unchanged. Existing `namespace_trace::fixture_exhausts_only_a_real_owned_child_namespace_verification_read` and `namespace_trace::fixture_causes_real_child_mode_and_namespace_capacity_refusals_in_owned_contexts` are already live controls, not pending activation.

The following deliberate external inputs **keep** their ignore attributes: `placement::selected_guest_body_uses_its_own_kernel_in_existing_profiles` in the acceptance binary; and `selected_environment_and_status`, `selected_attempt_waits_for_its_timeout`, `selected_attempt_resets_its_guest` in `infra/metal/tests/fixtures/attempt/tests/attempt.rs`, binary `metal-run-isolation-attempt-fixture::attempt`. The existing external driver explicitly selects them with `--run-ignored only`; their purposeful status 255/timeout/reset must not become an always-running regression suite. Merge their exact [placement additions](distill/test-placement-additions.json), the remaining new identities and the reviewed initial partition into runtime policy; retain original qualification placement and non-Metal/exclusive exec passthrough. Do not infer placement from an identity's absence or pin a brittle total count.

### RED and GREEN without the old Metal lease

No waived pre-implementation Metal check is a prerequisite to starting either step. There is no additional old-exclusive-runner compiler/test/native campaign, direct SSH test bypass or hidden alternate Metal execution. Prior outcomes and all post-adjustment **NOT RUN** entries in [checks.md](distill/checks.md) remain unchanged.

After removing its assigned pending markers, the source-ownership crafter records its own semantic RED with the real bootstrap/real Git/rsync fixture through canonical Lima routing:

```sh
cargo xtask lima run -- cargo nextest run -p xtask --test acceptance --features integration-tests -E 'test(=acceptance::metal_run_isolation::identity::same_branch_worktrees_keep_source_artifacts_and_evidence_disjoint)' --no-capture
```

That fixture has already reached the missing-binding assertion in DISTILL; the crafter reruns it against the actual current step state. GREEN runs the seven assigned identity/transfer cases in Lima, excluding the still-pending exclusive-mode case. Fixture SSH is the checked-in local transport input, never direct remote execution; it earns source/adapter evidence, not native qualification.

The integrated crafter begins with the existing collector and complete retained-v1/v2 fixture, without needing the not-yet-created wrapper or a native guest:

```sh
cargo xtask lima run -- cargo nextest run -p xtask --test acceptance --features integration-tests -E 'test(=acceptance::metal_run_isolation::collection::retained_v1_and_v2_commands_return_exact_status_on_read_only_repeat_collection)' --no-capture
```

Record the actual missing-v2 semantic assertion, not a compiler/prerequisite failure. Run supported source/record/flock/controller/qualification/wrapper fixtures and owning-crate protocol cases in Lima through `cargo xtask lima run -- cargo nextest run …` with exact selectors. This uses actual existing tooling and systemd where required; path-substitution fixture locks do not acquire the physical Metal barrier. Any platform prerequisite that Lima lacks remains an explicitly pending native item, never a passing skip or substituted oracle. The later namespace-operation/verification-read cases and live namespace fixture controls require the real approved kernel facilities even when a particular wrapper descriptor is host-pure. Actual immutable-root fault production requires its real filesystem/capability prerequisites. Do not manufacture fixture success to replace either boundary.

Compile/lint the changed Linux surface through Lima; for example:

```sh
cargo xtask lima run -- cargo check -p xtask --all-targets --features integration-tests
cargo xtask lima run -- cargo clippy -p xtask --all-targets --features integration-tests -- -D warnings
cargo xtask lima run -- cargo check -p overdrive-testing --all-targets --features integration-tests
```

Use scoped formatting and Bash/configuration parsing alongside these meaningful runtime checks. Metadata listing or compilation alone cannot satisfy GREEN. No broad test campaign, `nextest --no-run`, `cargo test` integration invocation, mutation run or fabricated pass/skip gate is added. Existing formal evidence/check definitions are retained; their hashes and approved recording provenance are checked without adding model work.

### Final implemented-path native proof

Only after the integrated implementation actually supports concurrent ordinary execution with all accepted isolation/completion mechanisms, prepare both participating callers from the updated source and use that updated ordinary protocol for remaining native acceptance. Do not run an old-exclusive-runner test campaign first. Guest/dispatcher preparation retains its approved preparation owner, without on-demand wrapper builds or physical-host `kmod-load`. Older branches and an existing exclusive maintainer still retain their actual authority; do not bypass or interrupt them. If that authority prevents the new-path proof, report the real outstanding condition and keep native acceptance pending.

The integrated step's GREEN and independent approval require successful actual remaining native/owner cases and these **existing** external tooling E2E drivers, not just local compilation:

| Pre-authored driver | Invocation arguments | Decisive evidence |
|---|---|---|
| `infra/metal/tests/worktree_journey.sh` | `CHECKOUT_A CHECKOUT_B OUTPUT_DIR` | Updated actual worktrees, existing batched core suite, overlapping SH owners/distinct namespace and source identities, post-START controller loss, exact repeat collection and K1–K3 kernel/process/three-root survivor complements |
| `infra/metal/tests/admission_journey.sh` | `CHECKOUT OUTPUT_DIR` | Actual systemd stop/finalizer order, exclusive administrative wait, unadmitted ordinary waiter, actual owner/lock/root observations |
| `infra/metal/tests/placement_journey.sh` | `CHECKOUT OUTPUT_DIR` | Actual selected ELF/helper/environment, fresh retry, default/ci adapter inheritance, supported timeout/guest reset, exact native/nested/operator/owner selections, typed refusals and status-255/aggregate-100 distinction |

Each output directory must be new, empty and outside every input worktree. Reuse one paired capture for K1–K3; do not rerun the suite for each identity/error row or create a density/benchmark campaign. The drivers' explicit `--exclusive` maintenance/fixture/status controls exercise the approved boundary after replacement, not a forbidden pre-implementation old-runner check. Retain raw first-failure diagnostics, source/tool/kernel/dirty-state attribution and each actual command outcome. The driver-internal expected 100/255 results are assertions about deliberately failing selected bodies; only the enclosing successful E2E assertions establish their contract.

The native reviewer independently assesses the final implemented-path evidence and all zero-delta invariants. A failed or unrun native item prevents claiming the feature complete; it never retroactively blocks starting authorized implementation under the DISTILL waiver. Native evidence contradicting a DESIGN premise triggers the repository's native falsification gate rather than a weakened test/contract. No native runtime result, implementation, DES event or commit was produced while authoring this roadmap.

## Wave: DELIVER / [REF] Implementation and verification disposition

Implementation is committed and technically reviewed. **The feature is not finalized: required native acceptance remains pending.** This disposition does not weaken AC-1–AC-7 or K1–K3, waive their outcomes, or treat the user’s restriction on exclusive-Metal tests as proof of those outcomes.

| Boundary | Actual disposition |
|---|---|
| Preparation | Approved design, acceptance scaffolds and retained construction/model evidence committed in `0e8ae265c`. |
| Source ownership | `ee0bbd658`; seven assigned identity cases passed through Lima. Independent [review](deliver/review-01-01.md) APPROVED. |
| Concurrent runner implementation | `2ba223220`; reported Lima evidence includes 32 acceptance cases, 24 fixture-library tests and two transitioned qualification cases. Scoped compile/lint/syntax/format checks passed. Independent [technical review](deliver/review-01-02.md) APPROVED; native acceptance remains PENDING. |
| Test-support lints | Equivalent option idioms committed separately in `4bc17cc74`; canonical Lima all-target xtask Clippy passed. |
| Native preparation | The author reported guest-kit preparation through the updated ordinary runner on both checkouts. This is preparation evidence, not execution of the paired verification bodies. |
| Ordinary-pair capture | Two retained attempts ended before START on maintenance admission. The second attempt’s raw timestamps show sequential waits. Neither capture proves concurrent guests, K1–K3, physical-kernel complements or survivor cleanup. [First attempt](deliver/native-evidence/ordinary-concurrency-attempt-2026-11-08/README.md); [second attempt](deliver/native-evidence/ordinary-concurrency-attempt-2026-11-08-attempt-2/README.md). Reported numeric status and record-derived versus inferred owner details are distinguished in those receipts. |
| Other native verification | Admission and placement external drivers were not run; their native flows include exclusive admission. Native-dependent Quint owner-conformance cases could not complete on ARM Lima because native preflight correctly refused the architecture. Existing admission-timeout and no-sync-mismatch cases passed. |
| User verification restriction | No mutation testing. Do not queue old exclusive-Metal test campaigns or bypass owners. Unrun checks remain unrun; there is no successful native completion claim. |
| Formatting | Step-owned Rust formatting passed; the author reported existing full-workspace formatting drift in untouched `guest_peer.rs` and `xtask/tests/acceptance.rs`. |
| Documentation | Operator guide and engineering post are independently [APPROVED](deliver/review-documentation.md). Content explicitly preserves the pending native proof. No website publication or deployment occurred. |

Documentation validation, as observed and reported by the documentation author: typecheck, lint, Next/OpenNext builds, one-index discovery, scoped links, and focused local Worker page/navigation/blog/search/export checks passed. Existing blog, MCP and analytics acceptance scripts stop on the same deterministic-simulation search-ranking assertion; the author reproduced that assertion on an untouched `git archive HEAD:website` baseline. Later analytics assertions remain unexecuted. Standalone command logs were not retained; the review distinguishes these author-reported results from independently rerun checks.

Both roadmap steps have actual CLI-written DES phase traces. Those traces record executed implementation phases; they do not discharge the pending native acceptance criterion. The remaining release gate is the required source-attributed native proof through the implemented ordinary runner, with real overlap and the prescribed kernel/resource/cleanup complements. Do not mark the feature finalized or claim the original service feature resumed. All unrelated dirty work is preserved.
