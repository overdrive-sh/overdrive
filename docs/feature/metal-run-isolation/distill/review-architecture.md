# Consolidated upstream review — architecture and contract fidelity

Date: 2026-11-08. Role: `nw-solution-architect-reviewer`. Iteration: 1.

**Technical verdict: APPROVED for the DISTILL authoring handoff.** There are no blocking architecture or interface-contract findings. This verdict does not claim implemented v2 behavior, complete native execution, or a successful delivery capture.

## Authority and review boundary

The authority is the exact P1–P5 contract in [feature-delta.md](../feature-delta.md), including its recorded user instruction **“Approve P1–P5”**, the five decision-specific ADRs, and [DESIGN review iterations 2 and 3](../design/review.md). Historical proposal labels do not revoke that explicit approval. The recording-only `ConfStep` correction is independently approved and does not reopen the production protocol.

Reviewed DISCUSS outcomes AC-1–AC-7, DESIGN interfaces/record formats/error mappings/lifecycle gates, DEVOPS E1–E9 and coexistence, [the release slice](../slices/slice-01-concurrent-worktree-verification.md), [environments.yaml](../environments.yaml), [scenario bindings](test-scenarios.md), [the self-audit](completeness.md), [checks and waiver](checks.md), and [exact placement additions](test-placement-additions.json). Inspected the authored Rust acceptance bodies, owning-crate conformance driver, transport/resource fixtures, three external tooling drivers and read-only observer. Current runner source was consulted to distinguish retained contracts from future implementation requirements.

Loaded review instructions: `nw-review` and `nw-sar-critique-dimensions`; the installed `nw-solution-architect-reviewer` definition; repository AGENTS/CLAUDE and applicable development, design, testing, verification, Rust and domain-naming rules. The task-specific native Markdown artifact and local-only review instructions govern this review; no additional reviewer, implementation agent or later wave was dispatched.

## Findings and dispositions

No critical, high or other mandatory finding is raised. No `SPECIFICATION_AMBIGUITY` remains in the reviewed public, cross-process, record, ownership or semantic-error interfaces. Private fixture helpers, syscall scheduling and conformance-driver decoding are DISTILL-owned implementation choices and are not treated as DESIGN gaps.

**praise:** The authoring preserves a useful evidence separation: actual controller producer tests establish why an ABORT is selected, the protocol driver exercises the approved ABORT/START owner boundary, and retained-record tests establish reader behavior. None is represented as proof supplied by one of the other layers. See [controller.rs](../../../../xtask/tests/acceptance/metal_run_isolation/controller.rs), [the driver](../../../../xtask/src/metal_run_isolation_conformance.rs), and [collection.rs](../../../../xtask/tests/acceptance/metal_run_isolation/collection.rs).

## Exact contracts and lifecycle meanings

| Accepted contract | Authored evidence and assessment |
|---|---|
| P1: canonical linked-worktree identity; physical-root/full-ref clone fallback; full SHA-256 key plus full binding equality | `identity.rs:8` computes the prescribed byte input independently; the eight identity cases cover equal branch ownership, linked branch switch/detachment, clone separation, sanitisation collisions, override binding/refusal, retained legacy contents, repeat sync and dirty-source pull. No identity service or automatic cache adoption is introduced. |
| P2: maintenance SH/EX then checkout EX, one admission budget, invocation-specific owner metadata and exactly 18 durable arguments | `admission.rs:39` reads actual named lock descriptors and checks the survivor; `:86` distinguishes timeout causes; `:220` exercises one real budget across both locks. `NativeUnit::spawn` at conformance-driver line 179 passes original values 1–15 and only the three approved appended values. The two transitioned qualification identities retain explicit exclusive authority and their existing diagnostics/cancellation/artifact assertions. |
| P2: preparing authorises controller transfer/checks; START is one-shot; post-START controller loss does not cancel the durable command | Prestart controls at `admission.rs:125`, replay refusal at `:158`, and controller loss at `:39` retain these meanings. The full-bootstrap cases call actual source transfer/preflight and inspect retained runner terminals. Running is not interpreted as guest READY or test success. |
| P3: existing CID bands and actual bound listeners inside anonymous local-mode vsock namespaces | Native placement cases call the real proposed adapter and observe actual namespace identity and mode; denied capability, read-only private sysctl view, scoped namespace capacity and actual verification-open exhaustion exercise the four declared failure steps. No allocator, preclaim holder, CID window, product wire field or public product method is added. |
| P3/P5: exactly three invocation roots; uid 0/gid 4200/mode 0710 traversal for launcher roots; preserved source/build/foreign-run complement | `finalization.rs:38` drives the exact finalizer entry and checks both repeated cleanup and preservation. `:95` produces a real deletion refusal separately at each root. External observations inspect actual root metadata, process/cgroup ownership and surviving invocation resources; they do not infer cleanup from main-process exit. |
| P4: nextest retains selection, exact ELF/argv/helper/env, attempt/retry/profile/report/result ownership | Host-pure/pass-through/refusal cases in `placement.rs` preserve the original chain and avoid a dispatcher dependency for host-pure bodies. The standalone actual Cargo fixture checks helper execution and property/attempt inputs; its external driver checks two distinct guest boot IDs for retries and body-255/aggregate-100. Fresh-per-attempt groups and existing explicit-suite batching remain separate. |
| P4: closed exact placement, no absence-means-host, only the approved `run --exclusive` addition | Runtime policy is compared with the reviewed partition and explicit DISTILL additions at `placement.rs:328`; malformed/duplicate/unknown data and unprepared contexts refuse before the body. Ordinary grammar/config/artifact refusals are driven through the developer commands. Non-Metal and explicit exclusive execution preserve the original chain. |
| P5: actual terminal distinct from unit stop and completion receipt; legacy v1 unchanged; v2 collection read-only | Reader cases distinguish legacy/current records, actual command codes, stopped-owner gating, malformed metadata/completion, cleanup failure and missing-terminal precedence. The real-owner driver uses actual systemd stop and captured finalizer code; the external admission journey observes exclusive maintenance only after predecessor stop and root cleanup. No collector cleanup, result reconstruction, reboot recovery or scavenger is introduced. |
| AC-7: direct-native launcher evidence and specifically named nested/operator/owner contexts retain their subjects | `placement_journey.sh` selects the existing direct-native launch/confinement body, named nested body and exact init operator/owner bodies. The timeout/reset fixtures first distinguish the actual guest boot identity from the physical launcher. No nested result is credited as direct-native proof. |

The authored paths remain developer tooling paths. They do not change Overdrive allocation Running/Stable/readiness/health, reconciler ownership, product persistence or product CLI behavior. No Rust test spawns the Overdrive production binary or becomes an expectation runner.

## Semantic error mapping gate

The approved interfaces use finite shell marker/record fields. The table maps every designed semantic failure to its approved representation and the authored assertion. These are pre-authored assertions for future implementation; absent v2 paths are not described as already implemented.

| Designed failure | Approved representation/status | Authored check |
|---|---|---|
| Invalid discovery, bare/non-Git source or detached ordinary clone | Runner `checkout_identity_invalid` / 64 | `identity.rs:148`; no remote complement delta |
| Invalid override, including symlink component | Runner `remote_subdir_invalid` / 64 | `identity.rs:148,186` |
| Nonempty unbound, malformed or differently owned checkout | Runner `checkout_identity_conflict` / 78; same tagged ABORT/terminal when registered | `identity.rs:186`; `admission.rs:125`; closed retained-reader table |
| No-sync binding/source mismatch | Runner `source_mismatch` / 78, retained same code before START | Full-bootstrap `controller.rs:10`; driver `no_sync_refusal`; prestart controls/reader table |
| Maintenance or own-checkout deadline | `maintenance_timeout` or `checkout_timeout` / 75 | `admission.rs:86,178,220`; real-owner timeout actions |
| Preparation, including unavailable transfer rsync | `runner_preparation_failed` / 75; immediate `prerequisite=rsync`; no START/package install | `identity.rs:265` and full-bootstrap `controller.rs:34`; no-sync control proves unused rsync is unnecessary |
| Native/source/owner prerequisite | `preflight_failed` / 78 and exact approved `probe` | All eleven probe partitions at `qualification.rs:55`; actual preflight adapter and retained ABORT path in conformance |
| Unsupported ordinary command or caller env grammar | `unsupported_ordinary_command` / 64 | `placement_journey.sh` safe refusal inputs |
| Replacement config/runner/debugger or archive reuse | `execution_override_refused` / 64 | External placement grammar cases; no fallback |
| Foreign artifact/manifest/target scope | `artifact_scope_mismatch` / 64 | External driver-owned foreign-path cases |
| Exclusive outside run | Existing Clap parse error / 2; bootstrap `invalid_mode` / 64 | `identity.rs:287`; external public CLI case |
| Invalid run/dispatch context | Test `invalid_run_context`, field `run_id`, `execution_class` or `source_owner` / 78 | `placement.rs:158` |
| Invalid, duplicate or missing placement | Test `placement_invalid`, `placement_ambiguous` or `test_unclassified` / 78 | `placement.rs:158,223`; body absence asserted |
| Namespace setup failure | Test `namespace_setup_failed`, exact `step` and positive decimal errno / 70 | `placement.rs:300,371,420`; parent=EPERM, child_mode=EROFS, local=ENOSPC, verify=EMFILE producer fixtures |
| Missing required dispatcher/context/guest input | Test `lane_not_prepared` / 78; no on-demand build/body | `placement.rs:158`; missing guest input probe partitions |
| Image/VMM/peer execution failure | Test `guest_execution_failed` with original GuestKit cause / 70 at wrapper; unchanged nextest aggregate mapping | Actual boot-guarded guest reset fixture and external error/cause assertions |
| Owned-root removal failure | Completion `kind=runner error=cleanup_failed root=<guest_staging|vmm_runtime|command_tmp>`; collector 74 | Real immutable-file producers at `finalization.rs:95`; terminal retained and actual failing root checked |
| Corrupt/missing retained artifact, ambiguous metadata or invalid completion | `run_artifact_io` / 74 | `collection.rs:77,146`; ordered/LF/schema, identity, enums and numeric-domain cases |
| Invalid ID, unknown run or lost supervisor | `invalid_run_id` / 64, `run_not_found` / 66, `supervisor_lost` / 70 | `collection.rs:77,219`; real main-owner loss trace; missing terminal takes precedence over missing receipt |
| Existing ABORT/EOF/launch/log failures | Existing retained finite errors; established `run_not_started` / 75 or artifact / 74 reader mapping | `admission.rs:125` and `collection.rs:146`; current `lease-holder.sh:285` reader mapping consulted |
| Actual selected command/body status | Command terminal 0–255; nextest body failure retains nextest aggregate 100 | Read-only v1/v2 status table, exact 255 owner trace, standalone attempt and external command-255 cases |

Unknown ABORTs remain `launch_aborted`; unknown terminal/record values remain artifact errors. A numeric code equal to a runner status is not used to erase command/runner kind. No generic Rust envelope or diagnostic string is substituted for the pinned semantic distinction.

## Owner-conformance honesty

The driver is test-only under the owning crate's `src/`, wired with `#[cfg(all(test, feature = "integration-tests"))]`. Its world owns actual transient units and captured current durable/finalizer code. No always-false compilation gate is present. Its `switch!` maps every recorded tag to the approved controller, durable-owner, source-transfer, systemd, collector or observation boundary; Idle reads actual unit observations. There are no empty or bookkeeping-only protocol action handlers.

The exact `{tag, value: {actor, value}}` codec and `Config::nondet` path match the independently approved recording contract. A read-only local comparison confirmed all thirteen permanent named runs have exactly matching pins; the driver also declares four randomized 24-step traces and a separate illegal-control state-boundary case. No second spec, additional hazard family or model check is introduced.

| Projected fact | Actual source inspected |
|---|---|
| `runs[r].p` | Validated phase/metadata; real manager MainPID/ActiveState/ControlGroup; discovered barrier/checkout descriptor targets and flock modes; actual selected PID/cgroup for running |
| `runs[r].term` | Validated actual terminal kind and approved code/error fields |
| `runs[r].receipt` | Validated actual completion kind/error/root fields |
| `second_tree` | Actual binding contents and canonical source paths |
| `source_revision` | Validated actual source marker plus transferred controlled source bytes |
| `maintenance` | Real exclusive descriptor, actual selected maintenance-command output/PID and manager stop |
| Current Collect result | Actual returned collector process status, consumed once for that action's comparison |

The driver excludes `.ctl`, `.checked`, `.ever`, `.starts`, `.desc`, `.roots`, `conf_step` and result-history ghosts from production projection. Its handles and input paths address actual observations; they are not shadow state used as an outcome oracle. Resource complements are inspected independently after actual stop. The short contended flock input is identified as an injected primitive failure; the separate unmodified two-lock deadline case carries the actual elapsed-budget claim.

## Equal-weight subtractive assessment

The added scope is justified by the original accepted criteria: identity/transfer/refusal cases defend AC-2/3; actual admission/control/conformance defends AC-1/3/6/7; namespace, placement and selected-attempt fixtures defend AC-4/7; finalizer/collector/owner-stop cases defend AC-5/6. The three external journeys supply the source/guest/kernel/process composition that the narrower Rust fixtures cannot credit.

No new scheduler, guest pool, allocator registry, recovery controller, persistence subsystem, product API, recurring lane, benchmark, synthetic evidence writer or mutation campaign appears. Namespace fault and finalizer timing machinery is test support at already-approved driven boundaries. Finite cheap reader/input tables do not multiply native guest boots. The explicit 54-row addition set has no duplicate identity; the two transitioned existing identities retain their partition. No mandatory scope expansion or additional mechanism is warranted by this review.

## Verification and limits

This review ran only read-only local source/JSON comparisons: 54 explicit placement additions plus two transitions, no duplicate identity, thirteen model-run/pin matches, no always-false driver cfg, and matching per-case Contract Shape declaration counts in the inspected acceptance modules. No Metal, SSH, native, Lima, Cargo, guest, model-check or mutation command was executed.

The user's **“STOP RUNNING TESTS THAT REQUIRE THE METAL LEASE”** instruction and explicit pre-implementation waiver remain in force. [checks.md](checks.md) truthfully preserves the earlier Clippy pass, seeded missing-P1-binding RED, passing verification-read fixture, failed permission fixture, subsequent local adjustment and all remaining checks marked NOT RUN. This review does not convert the 15/15 author audit or those earlier captures into a post-adjustment compile/runtime pass. Full owner replay beyond the current P1 RED, effective default/ci execution, producer fault execution and external two-worktree/native capture remain implementation-verification work.

## Verdict and remediation disposition

**APPROVED.** Blocking findings: 0. Required remediation: none. Accepted P1–P5 and the independently reviewed recording correction remain intact. The handoff may proceed with its stated verification waiver; no new approval, native rerun, contract relaxation or architecture expansion is required by this review. The original service-feature DELIVER remains stopped, and all pre-existing dirty work is preserved.
