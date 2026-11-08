# Metal run isolation — independent DESIGN review

## Review metadata

- Date: 2026-11-08.
- Role: `nw-solution-architect-reviewer`.
- Iteration: 1.
- Technical verdict: **NEEDS_REVISION**.
- User approval: **PENDING** for P1–P5. This review neither supplies that approval nor accepts the proposed ADRs or architecture references.
- Scope: the proposed `metal-run-isolation` contracts, their construction evidence, and narrowly related current execution paths. The stopped `service-allocation-readiness-and-ending` implementation is excluded.
- Artifact authority: `feature-delta.md` is the proposed interface contract; ADR-0277–0281 each own their architectural decision. The native captures establish construction facts, not implementation of the proposed v2 runner.
- Skills loaded: `nw-review` and `nw-sar-critique-dimensions`; reviewer role definition loaded from the installed nWave agent directory. Repository `AGENTS.md`, `CLAUDE.md`, and applicable design, development, testing and verification rules govern over role defaults.
- Roadmap review: not applicable; this feature has no roadmap. No roadmap approval is implied.

## Findings

### F1 — issue (blocking): the claimed source-reviewed placement partition omits real socket owners in `overdrive-sim`

**Severity:** high. **Category:** contract consistency / construction-evidence completeness. **Owner:** DESIGN, P4.

The inventory contains all discovered identities, but its `host-pure` classification does not establish the promised disjoint resource boundary for six current Gateway Application acceptance identities. The two `acceptance::gateway_relay_exchange::*` identities are **not ignored**. Four `acceptance::gateway_application_lifecycle::*` identities are ignored but expressly remain in the reviewed universe and are selectable through the supported ignored-test policy. Every one is currently classified `host-pure` with no guest context.

The exact current paths are:

1. Existing `cargo nextest run -p overdrive-sim --test acceptance --features integration-tests` selection → `tests/acceptance.rs:211` → `gateway_relay_exchange.rs:514-524` or `:851-856` → `Rig::start(122, seed)` or `Rig::start(123, seed)` → deterministic addresses at `:113-116` → real `std::net::TcpListener::bind((vip, SERVICE_PORT))` at `:138`, retained in `Rig::_frontend_listener` at `:108`. The first selected seed is 0. Two invocations therefore name the same Service frontend `127.122.254.1:8080` or `127.123.254.1:8080`. The real gateway listener is also configured from the same deterministic public address (`:185-192`, `:249`); its production bind is `overdrive-gateway/src/application.rs:889`.
2. Existing ignored-test selection → `tests/acceptance.rs:207` → `gateway_application_lifecycle.rs:43-45,74-116` → `Harness::only(...).run(seed)` → `src/harness.rs:747-752` → the actual evaluator in `src/invariants/gateway_application.rs:1044-1062` → `Rig::start` at `:345-362`. Its retained real frontend listener binds `127.<120|121>.254.<seed % 250 + 1>:8080`; the public-address PID adjustment at `:304-308` does not qualify that frontend. The existing source itself states that concurrent same-seed frontend use is unsupported (`:18-27`).

The ordinary grammar also explicitly admits `cargo run -p overdrive-sim --bin dst -- ...` unchanged on the host (`feature-delta.md:335-338`). Its existing `--only` parser and `Harness::only` path (`src/bin/dst.rs:64-89`) can select those same two socket-owning evaluators. A nextest identity table alone cannot classify this program entrypoint as universally pure.

This is the same resource distinction correctly recognized for Gateway tests in `design/placement-inventory.md`: a Sim relay does not make the real fixed-address listeners process-private. P4 currently requires retaining **exactly the same partition** (`feature-delta.md:360`), whereas the outcome requires zero cross-invocation resource collisions. The host-pure declaration and its rationale need revalidation before they can be treated as closed construction evidence.

**Evidence limit:** I did not reproduce an OS bind failure or claim a newly proven production runtime defect. Review authorization excludes remote mutations, and no local Lima instance exists (`limactl list --json`). The source facts and contradictory placement declaration above are established; a failure schedule remains unproven. Do not use this finding to change product code, invent a Sim seam, or expand the original Gateway feature.

**Required disposition:** return these six identity placements and the admitted `dst --only` path to DESIGN. Perform a bounded confirmation through the existing entrypoint before choosing a remedy; then pin their actual invocation resource boundary and update the partition/ordinary-entrypoint contract consistently, or retain an honest unresolved construction gate. Reuse the existing isolation options; this review does not authorize a broker, scheduler, address-allocation protocol, or whole-run outer guest. Do not classify this as a product control-plane ordering defect or change its invariants.

### F2 — issue (blocking): the required formal-protocol artifact is absent for the changed admission and completion protocols

**Severity:** high. **Category:** mandatory DESIGN validation. **Owner:** DESIGN, P2/P5.

`.claude/rules/design.md:369-381` requires a checked Quint specification for a changed concurrent, ordered or crash-sensitive protocol, explicitly including leases, admission and multi-step teardown. Every DESIGN also requires a `Formal protocol model` section. The proposed feature has no such section or referenced model/evidence; `feature-delta.md:469` instead says no formal model is requested.

The applicable changes are already explicit in the proposal: a service now holds shared/exclusive maintenance admission and checkout exclusion across controller source transfer and one START (`feature-delta.md:267-288`); an exclusive maintainer additionally waits for admitted units' stop/cleanup (`:288`); and new-format collection depends on terminal publication, unit stop, cleanup and a separate persisted completion receipt (`:365-394`). These are changed shared-owner orderings even though their primitives and service owner are reused. The native shared-flock and nextest-timeout constructions prove useful individual traces, not exemption from the repository's separate artifact gate.

**Required disposition:** the architect must resolve the applicable repository gate explicitly: supply the narrowly scoped protocol model, checked evidence and pinned existing owner projections required by the rule, or surface and record an explicit user waiver of this specific requirement. Do not invent a runtime recovery/persistence subsystem, add product APIs merely for modeling, or model unrelated product lifecycle protocols. No counterexample or runtime defect is asserted here. Pending P1–P5 approval is separate from a waiver of this gate.

### F3 — issue (blocking): the newly named source-transfer prerequisite lacks an exact failure mapping

**Severity:** high. **Category:** `SPECIFICATION_AMBIGUITY` at the typed-error contract gate. **Owner:** DESIGN, P2.

P2 deliberately removes ordinary package installation and says that missing remote rsync is a prerequisite failure (`feature-delta.md:269`). The current affected entry path is `bootstrap.sh:518-525`: the ordinary sync/run preparation checks rsync and currently installs it. The proposed replacement failure occurs before the selected command starts, potentially while its durable registration and controller-owned source transfer are active. It is therefore neither an actual selected-command result nor an image/guest execution failure.

The finite `preflight_failed` probe payload excludes rsync (`feature-delta.md:424`), while `runner_preparation_failed` is currently mapped to invocation-owned filesystem/context preparation (`:423`). The source-transfer row does not assign this prerequisite to either approved code or the existing ABORT outcome, state its controller exit, or state its retained/collected representation after registration. The accepted tagged ABORT alternatives are finite (`:282`). An implementer must not choose an unrecorded mapping or invent a public probe/code merely because it compiles.

**Required disposition:** pin this condition's exact existing-or-new approved representation, status and pre/post-registration retention mapping in the authoritative interface contract. A sentence explicitly reusing a suitable existing approved representation may suffice; no new error variant or subsystem is requested by this review. Do not pause unrelated reviewed design work, but do not implement the affected failure path until its contract is pinned.

### F4 — issue (blocking): component Contract Shape declarations are incomplete

**Severity:** high under the reviewer's mandatory Effect Isolation Compliance gate. **Category:** DISTILL handoff precision. **Owner:** DESIGN.

The `Owner and Contract Shape` column (`feature-delta.md:225-235`) provides useful ownership and bounded-universe prose, but most executable component rows do not declare one of the required shapes: `pure-function`, `bounded-change`, or `unbounded-preservation`. Examples are native preflight, the nextest adapter, the existing guest executor, fixture preparation, and native fixture constructors. The pure derivation and transfer shell row also combines two different effects without classifying their component contracts. Static data and documentation can be explicitly declared non-executable rather than assigned a fictional executable contract.

**Required disposition:** finish the per-component classifications and state the declared delta within the already named universe for the effectful components. Retain the existing finite host allow-set and other-invocation complement. This is a bounded declaration fix, not permission to prescribe private function signatures, split modules, introduce ports, or add an implementation framework.

## Positive and subtractive review

- **praise:** replacing the userspace CID-window proposal with the inspected kernel local-mode vsock namespace removes an allocator, claim records and a separate listener-isolation mechanism while preserving the zero-collision outcome. The paired GuestKit capture exercises actual production VMM launch, rather than relying solely on direct ioctl success.
- **praise:** failed native attempts are disclosed. The fixture-parent correction retains 128 MiB, production UID/GID/device groups, seccomp/no-new-privs/Landlock and the data assertion. Native evidence is kept direct; named nested cells receive their narrower existing credit.
- **praise:** actual command terminals remain separate from cleanup completion, and collection remains read-only. The actual nextest 100/body 255 distinction and the original supervisor-loss behavior are stated accurately.
- No need was established for a CID allocator, persistent guest pool, framework, new control-plane Sim seam, automatic stale-run scavenger, reboot recovery or broad-suite rerun. None is recommended.
- The five ADRs are individually decidable and contain decision context, alternatives and consequences, with interface specifications remaining in the feature delta. Brief/C4 additions are explicitly proposed rather than silently accepted.
- Direct-nextest boot overhead is real and disclosed. The material ordinary/exclusive compatibility and fresh-per-attempt lifetime decisions must remain user decisions; the reviewer does not substitute its preference.

## Interface and failure-contract audit

The following representations are **pinned proposals**, not user-approved contracts or implemented v2 behavior. F3 is the identified missing semantic mapping.

| Designed condition or interface | Pinned proposed representation | Assessment |
|---|---|---|
| Git discovery / detached ordinary clone | `checkout_identity_invalid`, 64 before remote mutation | Exact; linked-worktree identity remains branch independent |
| Override syntax/scope | `remote_subdir_invalid`, 64 | Exact; canonical physical directory also owns lock identity |
| Unbound nonempty, foreign or malformed binding | `checkout_identity_conflict`, 78; approved tagged ABORT if registered | Exact; full identity comparison accompanies SHA-256 key |
| No-sync source/binding mismatch | `source_mismatch`, 78 and retained runner terminal | Exact |
| Maintenance/predecessor-cleanup deadline | `maintenance_timeout`, 75 | Exact; one admission budget |
| Own-checkout deadline | `checkout_timeout`, 75 | Exact; distinct from another ordinary worktree |
| Invocation filesystem/context preparation | `runner_preparation_failed`, 75; tagged ABORT | Exact for its declared set; rsync prerequisite mapping is F3 |
| Native/source/owner prerequisite | `preflight_failed`, finite `probe` enum, 78 | Exact; immediate probe context does not imply a new product readiness state |
| Unsupported ordinary command / config override / foreign artifacts | `unsupported_ordinary_command` / `execution_override_refused` / `artifact_scope_mismatch`, 64 before START | Finite proposed codes; no implicit exclusive fallback |
| Invalid exclusive mode | Public Clap parse 2; bootstrap `invalid_mode`, 64 before registration | Exact boundary distinction |
| Wrapper context | `invalid_run_context`, finite `field` enum, 78 before body | Exact |
| Placement syntax / duplication / absence | `placement_invalid` / `placement_ambiguous` / `test_unclassified`, 78 | Fail-closed set pinned; source-correctness issue remains F1 |
| Namespace setup | `namespace_setup_failed`, finite `step`, numeric errno-or-none, 70 | Exact; no global-CID fallback |
| Missing dispatcher/input | `lane_not_prepared`, 78 | Exact; no opportunistic wrapper Cargo build |
| Existing fixture/peer failure | `guest_execution_failed`, retained existing typed `GuestKitError` cause, wrapper 70 | No new product Rust error surface required; nextest keeps aggregate ownership |
| Owned-root cleanup failure | `completion.v1 kind=runner error=cleanup_failed`, finite `root`, collector 74 | Exact; actual command terminal retained |
| Corrupt/missing artifact | Existing `run_artifact_io`, 74 | Exact; incomplete v2 cannot become legacy v1 |
| Invalid ID / absent run / lost supervisor | Existing `invalid_run_id` 64 / `run_not_found` 66 / `supervisor_lost` 70 | Exact; no status reconstructed from systemd |
| Existing ABORT/EOF/launch/log failures | Existing finite terminal values and existing 75/74 handling | Existing compatibility retained; not authority to improvise F3's mapping |
| Actual selected command/body result | Actual 0–255 terminal/peer code; nextest owns its aggregate result | Correct distinction between body 255, nextest 100 and supervised command 255 |
| Checkout binding | Ordered `.overdrive-metal-checkout.v1`; exact format, kind/key/base64 identity | Sufficient interface shape; no automatic legacy adoption |
| Durable launch/control | Original 15 values plus tree-lock, tree-owner and execution-class; finite START/ABORT controls | Pinned; shared-owner protocol validation remains F2 |
| New run metadata / collection | Exactly one matching v1/v2 filename-format pair; execution class appended; checkout copy and completion receipt | Compatibility and record ownership explicit; not implemented yet |
| Selected test execution | Exact binary ID/name plus actual executable/argv, helpers and property/attempt environment | Interface pinned; F1 prevents treating the current host partition as closed |

The proposal adds developer CLI/config/environment and persisted runner formats, while changing no product public Rust trait, enum, method, wire format or allocation/readiness meaning. Private fixture plumbing remains the implementer's/test author's responsibility; this review finds no reason to pin those private signatures.

## Evidence checked independently

### Current execution and owner paths

Read the current `xtask/src/main.rs` Metal parser/dispatcher, bootstrap identity and sync/run/pull paths, durable service registration/START/signal/terminal/collector paths in `lease-holder.sh`, native preflight, nextest groups, relevant guest fixture/port owners, the real init beacon caller, and the specific socket-owner paths in F1. In particular:

- Current bootstrap uses branch-derived remote directories, a global exclusive lease and a controller checkout-lock release/reacquire window. P1/P2 propose real changes to those contracts; they are not described as already delivered.
- `lease-holder.sh:230-237` already treats activating/active/deactivating units as live. Its current metadata/terminal validators (`:102-185`) and collector (`:279-443`) are v1-only. v2 completion behavior is a delivery obligation.
- `guest_lane.rs:250-252,294-298` currently binds a separate fixed pre-cut beacon while advertising another held listener. Current production init reads and dials `token.ports.beacon` (`overdrive-init/src/main.rs:585-590`), supporting the proposed port correction without a product wire change.
- `GuestKit::boot` already grants its supplied parent group-4200 traversal (`guest_kit.rs:548-558`); the native test's parent preparation is bounded fixture fallout.

### Finite inventory

Parsed the retained native nextest JSON and proposed inventory independently. The `(binary_id, test_name)` sets are exactly equal: **4,640 identities, 94 binaries, zero missing, extra or duplicate identity keys**. Counts match: host 3,815; native 38; ordinary guest-group 776; operator cells 4; owner cells 7. These establish enumeration completeness only. F1 shows why they are not proof of classification correctness.

No broad test suite was run. No Metal command, remote mutation, production source edit, production test-body edit, commit or DES event was made by this review.

### Pinned kernel source

Verified `git ls-tree HEAD vendors/linux` identifies `028ef9c96e96197026887c0f092424679298aae8`. The retained official tag output peels v7.0 to that commit. Recomputed all five hashes in `kernel-source-provenance.json`; every captured file matches.

Read the cited source paths at that exact revision: vhost opener namespace retention (`drivers/vhost/vsock.c:688-690`), namespace-aware CID match (`:74-88`), atomic set-CID (`:788-824`), last-device release (`:753-785`), namespace reachability and child-mode locking (`include/net/af_vsock.h:279-323`), scoped bind (`net/vmw_vsock/af_vsock.c:781-832`), read-only current mode/private-parent child mode and inheritance (`:2824-2864,2923-2932`). The mechanism follows those sources. The checkout's uninitialized submodule is not misrepresented as inspected; recovered exact-commit source is retained with provenance.

### Native construction and result limits

- Paired raw GuestKit outputs and `g-paired-result.json` show different held local namespaces and guest boot IDs, the same CID 16785415, actual statuses 0 and overlapping ready intervals. The primitive capture separately demonstrates duplicate refusal within one namespace and survivor-listener retention. The physical before/after observer in the retained driver covers modules, nft rules, BPF program identities, routes, test cgroups and initial vsock modes. These are bounded construction observations, not an implemented two-worktree runner capture.
- Compared the increment-N native fixture copy against the repository body. The material construction change grants the existing caller-supplied run parent group 4200/mode 0710; diagnostic path/CWD/output changes do not remove its identity/confinement/data assertions. Raw N output contains passing initial/local cases with 134,217,728 guest bytes, production identity/filter details and data ports. It does not validate the unchanged original fixture as passing.
- Read increment O's complete durable log and repeat collection. Dispatcher preparation, actual guest environment execution and timeout/cleanup construction each report their measured expected 0 construction status. Both timeout nextest commands actually return 100. The no-finalizer case leaves owned roots; the existing-style unit with finalizer removes the specifically observed roots after descendants end. The construction driver checks VMM PIDs are gone. This is not a full v2 finalizer/collector implementation test.
- The outcome checker explicitly checked zero outcomes. It contributes no semantic collision assurance.

## Architecture dimensions

| Dimension | Assessment |
|---|---|
| Bias / simpler mechanism | Strong use of existing owners and the kernel namespace; no preference-driven new service. F1 requires revalidating an overbroad assumption that a Sim target is host-pure. |
| ADR quality | Five focused proposed decisions, viable alternatives and consequences, correct authoritative links. |
| Completeness | Identity, native applicability, cleanup ownership and most exact failure mappings are explicit. F1–F4 block complete handoff. |
| Feasibility | Native construction supports the central namespace/fixture/result mechanism. Current placement and required protocol validation cannot be called complete. |
| Priority | Removes the observed unrelated-run lease bottleneck. No speedup benchmark is needed; paired execution and ownership observations remain the correct delivery outcome. |

## Remediation and verdict

All four findings are open in iteration 1. There is no implementation remediation or prior disposition to inherit. Return the proposal to its DESIGN owner, keeping P1–P5 proposed. Resolve the bounded placement and source-transfer contracts, required protocol validation/explicit waiver, and component declarations; then return the resulting design and evidence for independent re-review. No new architecture is prescribed by this review.

**Technical verdict: NEEDS_REVISION. User approval: PENDING.** DISTILL/DELIVER and the stopped service feature must not proceed on this review. User acceptance of P1–P5 and technical approval are separate prerequisites.

## Iteration 2 — remediation review

Date: 2026-11-08. **Current technical verdict: APPROVED. User approval: PENDING for P1–P5.** The iteration-1 findings and verdict above remain the historical record. The dispositions below supersede their open status; technical approval does not accept the proposed contracts on the user's behalf or authorize downstream implementation.

Reviewed the updated authoritative feature delta, component table, placement partition and ADR references; `design/remediation.md`; `design/f1-disposition.md` and its native raw evidence/driver; the complete permanent Quint protocol, hazards, manifest, README and retained evidence; and the specific current parser, harness and socket-owner paths needed to test the remediation's necessity. No production test, remote command, model rerun, source edit or commit was performed during this re-review.

### Findings and remediation dispositions

| Finding | Iteration-2 disposition | Independent evidence and conclusion |
|---|---|---|
| F1 — six Rust acceptance identities | **CLOSED — corrected after reproduction** | Native run `eb2bc9a1a2f401aa7935d2a3` used the actual Cargo-reported acceptance ELF and unchanged exact selected libtest body. Raw output shows one invocation passed with status 0 and its concurrent twin failed at `gateway_relay_exchange.rs:138` with errno 98 / `AddrInUse`, status 101, at printed seed 0. The retained driver has no source rewrite, process suspension or scheduling seam. This establishes the resource-collision premise through the current entrypoint. Exactly the six source-traced identities now use existing `guest-group/cgroup-default`, retaining their exact names and two active/four ignored flags. No product fix or new resource mechanism was introduced. |
| F1 — proposed `dst --only` reachability | **CLOSED — reviewer claim withdrawn** | My iteration-1 statement that the current production CLI can select those Gateway Application evaluators was incorrect. `Invariant::from_str` at `src/invariants/mod.rs:958-971` searches only `Invariant::ALL`; both variants are omitted there. `Harness::catalogue` at `src/harness.rs:308-310` also uses `ALL` unless supplied a typed override. The typed Rust acceptance entry can reach them; the CLI's string parser cannot. The actual built CLI returned 1 with unknown invariant and no printed seed. Its unchanged host execution boundary is correct for the current reachable catalogue. No selector activation, CLI repair or program-mode change is required. |
| F2 — formal-protocol gate | **CLOSED — required model and evidence supplied** | Read the actual P2/P5 state machine, both hazard instantiations and all thirteen named runs. The manifest and tool-written evidence contain the same 18 checks: exhaustive Safety, two weak-fair progress properties, two expected hazard violations and thirteen evaluator scenarios. Recomputed all three manifest/source SHA-256 values and matched them to the recorded evidence; every recorded outcome matches its expected outcome and every trace reference resolves. The retained `verify-evidence` output also reports success. Exact owner observations/action boundaries and model-only ghosts are now pinned in the feature delta, with kernel/systemd/record assumptions and their native evidence explicitly separated from the model's claims. No waiver, new runtime subsystem, product accessor or Sim seam is needed. |
| F3 — missing-rsync semantic mapping | **CLOSED — exact contract pinned** | P2 and the typed-error table now identify missing source-transfer rsync as `runner_preparation_failed`, immediate `prerequisite=rsync`, status 75. Sync/pull release their checkout lock without creating a durable record. A registered run sends the existing tagged ABORT before START, retains `terminal.v1 kind=runner error=runner_preparation_failed`, undergoes P5 finalization, and valid collection returns the same code/75. The prerequisite remains diagnostic context rather than a new terminal field. Missing-terminal 70 and artifact/cleanup 74 precedence are explicit; `--no-sync` does not require unused rsync. This closes the specification ambiguity without inventing another code/probe or changing selected-command results. |
| F4 — component Contract Shapes | **CLOSED — declarations completed** | The table now separates pure identity derivation from effectful discovery/transfer and declares `bounded-change` for each executable owner: durable service/finalizer, collector, preflight, wrapper, group execution, preparation, native fixtures and existing guards. Every row names its permitted delta and universe, preserving other invocations and the finite physical-host allow-set. Static placement data and documentation are explicitly non-executable. The declarations do not mandate private function shapes, new ports or module decomposition. |

**Remaining blocking findings: zero.** No new implementation or architectural remediation is requested.

### Verification detail and limits

The inventory's identity/ignored mapping remains exactly equal to the original retained Linux nextest discovery: **4,640 identities with no missing, extra, duplicate or changed ignored keys**. Metadata contains 94 discovered binary entries; 87 contain test identities and seven are empty. Current placement counts are host-pure **3,809**, guest-group **782**, native-launcher **38**, operator **4**, owner **7**. The corrected six rows all select the existing `cgroup-default` context. The source distinction remains necessary even though the relay and lifecycle driven ports include Sim implementations: their frontend sockets are real. The already-approved group machinery and proposed per-attempt resource boundary cover the correction; the evidence does not justify a generalized allocator or changes to the underlying Gateway feature.

The formal protocol has a real `step` relation and named critical scenarios, not a definitions-only conformance target. Its P2 hazard makes a second ordinary owner inadmissible behind another ordinary holder; the retained counterexample reaches that state. Its P5 hazard exposes a command result at main exit while the unit still owns descendant/root obligations; that counterexample also appears in the retained trace. The ordinary rules preserve checkout exclusion, source stability during the command, one START, maintenance exclusion, honest supervisor loss and completion-gated collection within the stated finite abstraction. Progress is conditional on the documented weak fairness and terminating/failed owner operations; it does not promise termination of an infinite selected command or prove kernel behavior. Native process/root complements remain direct owner observations rather than driver-ledger projections.

The recorded formal run took 270.3 seconds overall, with all 18 expected outcomes matched; it ran on the local model-checking substrate, not the native test kernel. Its source/manifest hashes match the reviewed files. The disclosed unknown peak-memory measurement and cleanup-attempt diagnostics are not recast as measured resource guarantees. This re-review validates the retained evidence and its applicability; it does not claim a new model-check run or implementation conformance pass.

The proposed rsync mapping reuses the already specified proposed error code and ABORT alternative, with exact lifecycle/collector ownership. Other interface and typed-error conclusions from iteration 1 remain applicable to the updated proposal. No zero-host-mutation, source-fidelity, exact-status, native-evidence or other-invocation-cleanup invariant was weakened. Proposed ADR status, user approval and completed technical review remain separate facts.

### Positive and subtractive assessment

- **praise:** the remediation reproduced the selected test's collision through its unchanged real entrypoint and corrected only the six affected declarations. It also disproved the reviewer's broader CLI premise rather than building machinery around it.
- **praise:** the bounded model supplies the required protocol evidence using existing shell/systemd/kernel observations. It keeps resource-obligation ghosts out of production projections and does not grow a recovery architecture.
- The implementation remains the existing runner and guest groups with the proposed bounded extensions. No scheduler, broker, guest pool, product API, new CLI program mode, original-service change or broad native test campaign was added to close this review.

### Current verdict and handoff boundary

**Technical verdict: APPROVED.** F1–F4 are closed with the dispositions above. **User approval: PENDING for P1–P5.** The design may be presented for those explicit decisions; this technical verdict cannot substitute for them. After user approval, DISTILL/DELIVER must use the resulting exact accepted contract and prove the future v2 composition through its real entrypoints. This review neither resumes the stopped service feature nor supplies implementation or roadmap approval.

## Iteration 3 — model recording compatibility

Date: 2026-11-08. **Current technical verdict: APPROVED.** User approval of P1–P5 is now recorded in the authoritative feature delta under `User approval — 2026-11-08`, quoting **“Approve P1–P5”**, and supersedes the historical pending-approval statements in iterations 1 and 2. This iteration reviews only the recording compatibility correction; it does not reopen those decisions or the previously closed findings.

### Bounded scope and verdict

Reviewed `design/recording-compatibility.md`, `design/codec-compatibility/findings.md`, the complete retained recording diff, actual codec diagnostic source/manifest/lock and generated trace/driver reports, installed quint-connect 0.1.2 decoding/runner code, the current protocol recording assignments, and refreshed permanent evidence. The compatibility correction is necessary and remains confined to the sole model's existing write-only replay ghost. **Zero blocking or non-blocking findings remain in this bounded review.**

The exact reviewed representation is `ConfStep = { tag: str, value: { actor: int, value: int } }`, selected by the single static `Config::nondet` path component `runner_conf::runner_protocol::conf_step`. Existing semantic action names and integer actor/value meanings are preserved. The already-existing `Idle` action now records `tag="Idle"` with actor/value 0/0 when recording is enabled; it continues to preserve every modeled production field. No protocol action, production accessor, state, owner, port, CLI, error, hazard or manifest check is added or changed.

### Necessity and evidence verified independently

| Review question | Evidence and disposition |
|---|---|
| Is the old shape actually incompatible with the pinned library? | Workspace `Cargo.toml` pins `quint-connect = "=0.1.2"`. Read its installed `src/driver/step.rs:23-49,106-139`: a nonempty nondeterministic-pick path must resolve to a record with a string `tag` and a record/unit `value`. The old flat `{op, actor, value}` does not satisfy it. The retained actual public-runner negative case reports `Expected action to be a sum type variant` and zero calls reaching `Driver::step`. This is a reproduced codec failure, not a production protocol defect. |
| Could a handler-local rename fix it without changing the ghost? | No. Installed `src/runner/mod.rs` constructs private `Step::new` before `Driver::step`, and state deserialization/comparison also follows step decoding. A static nondeterministic-pick path cannot perform that record transformation. The corrected record is accepted directly; no fork, dependency update, extra codec layer or alternate state machine is required. |
| Is the model change precisely recording-only? | Reversed `recording-only.diff` in memory against the current model and independently obtained the exact iteration-2 model hash `268c7b584c9bb5ec68ff29d9efb959980e111cf90f18dc8157c06c15a8e03f2d`. The current hash is `f67ed362807560c981a60be33d127106f47329a290f5148add70fadcbd732b5e`. Erasing only the `ConfStep` declaration and `conf_step` next-state assignments leaves identical model text modulo whitespace. The thirteen named runs are byte-identical. Manifest and hazard hashes remain exactly those reviewed in iteration 2. |
| Does actual quint-connect decoding deliver the stated actions and picks? | Independently compared each retained public-driver report with its actual generated ITF: all **13 named traces / 119 states** match the exact tag and two integer picks. Split the public runner's randomized report by real init events and compared all **four randomized traces / 132 states** to their ITF. Every recorded tag also matches the corresponding real `mbt::actionTaken`. All **45 Idle transitions** decode actor/value 0/0, including transitions following non-Idle actions. Seed is **20261108**. |
| Did the formal gate remain intact? | Recomputed all current source/manifest hashes in permanent `evidence/summary.json`; all match. Its unchanged eighteen-entry manifest records **16 holds and two expected hazard violations**, with zero unexpected results. The retained actual `verify-evidence` log returns 0. Recorded safety and both progress checks retain **695,458 distinct / 19,473,450 generated states** under `REC=false`; recording remains constant after initialization in those instances, with a bijective encoding of the two initial choices. The disclosed preliminary CI-duration warning was not hidden by changing the manifest. |

These checks inspect retained actual evidence and current files. No new native run, broad model campaign, production test execution or source/model/test-body edit was performed by this reviewer.

### Oracle, interface and ownership assessment

The codec diagnostic is a trace consumer through the real library's public runner. Its `State = ()` expressly disables production state comparison; it logs decoded stimuli and does not implement admission, cleanup, result selection or a second protocol oracle. The companion diagnostic compares those decoded stimuli with generated ITF and, for randomized traces, the real MBT action label. This is appropriate evidence for the codec boundary and supplies no claim that the future Metal owner conforms to the model.

`conf_step`, assertion history and the previously identified model-only resource obligations remain excluded from production projections. Actual shell/systemd/kernel observations remain the source of the conformance driver's compared owner state. The recording clarification distinguishes a fixture playing the controller at the declared ABORT boundary from the separately required full-bootstrap tests of actual no-sync/missing-rsync producer decisions; neither fixture-owned prepared state nor copied validation logic is authorized. Private handler and fixture structure stays DISTILL-owned. No public phase-selection port, extra owner mechanism or test-only production seam follows from this correction.

The typed production error audit from the earlier approved review is unchanged: this correction introduces no designed production semantic failure or representation. The original library decoding diagnostic is test-support evidence, not a new Metal runner error. Actual command codes, finite runner errors, completion/error precedence, source fidelity, zero physical-host test-kernel mutation, native applicability and cleanup ownership retain the accepted P1–P5 meanings.

### Positive and subtractive assessment

- **praise:** the correction reproduces the real pinned-library failure before choosing its exact envelope, and verifies named and randomized decoding through the actual public runner, including the previously misleading Idle record.
- The sole existing model remains the oracle. The correction avoids an imported recording state machine, trace-rewriting service, dependency fork, extra formal checks or production interface expansion. No additional user decision is needed for this test/model recording representation.

### Current verdict

**APPROVED — model recording compatibility remediation.** P1–P5 remain explicitly user-approved; their technical approval is retained. Iteration-1 findings remain closed under iteration 2, and this bounded iteration adds no new finding or architecture requirement. DISTILL retains responsibility for honest production-owner conformance and its separate producer-path tests; this codec evidence does not mark that implementation or acceptance authoring complete. The stopped service-feature DELIVER is unchanged.
