# Independent DESIGN review — native E18 bridge population falsification

## Metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261004_e18_native_falsification` |
| Date | 2026-10-04 |
| Role | Independent solution architecture reviewer |
| Model | GPT 6.1 Sol, extra-high (`xhigh`) reasoning, as explicitly requested |
| Iteration | 1 |
| Verdict | **CHANGES_REQUESTED** — one HIGH documentary evidence finding |
| Roadmap disposition | **PENDING / NON-EXECUTABLE**; `approved_at=null` |
| Replacement architecture | None reviewed, chosen, or approved |

Loaded reviewer definition `/Users/marcus/.claude/agents/nw/nw-solution-architect-reviewer.md`
and skills `nw-sar-critique-dimensions` and `nw-roadmap-review-checks` from
`/Users/marcus/.claude/skills/`. The explicit role/model, native Markdown,
bounded scope, and unlimited remediation instructions override the definition's
legacy Haiku/YAML/two-iteration defaults. This review audits the factual/status
delta; it does not repeat a full architecture or DELIVER implementation review.

## Exact scope

The reviewed documentary changes are:

1. `docs/feature/netns-density-295/deliver/native-e18-bridge-capacity-falsification.md`
2. `docs/feature/netns-density-295/feature-delta.md`
3. `docs/product/architecture/adr-0114-node-local-shared-bridge-guest-network.md`
4. `docs/product/architecture/adr-0117-initial-shared-bridge-density-target.md`
5. `docs/product/architecture/adr-0124-bounded-shared-network-owner-recovery.md`
6. `docs/feature/netns-density-295/deliver/roadmap.json`

The review also reads the necessary preserved raw attempts, bounded harness
report, exact production attachment/rollback path, and executable/source
metadata mechanism. It changes only this review and the bounded read-only
process-identity receipt linked below. It does not change production, support,
tests, AGENTS, DES events, raw attempts, reviewed design artifacts, or any kernel
object. No commit, build, population rerun, mutation test, or replacement
topology/population choice is performed.

## Independently verified facts

The nine-file attempt at
`target/benchmarks/netns-density-295/E18/attempt-08-04-20261004/e18-t1-base/`
has method N=16,384/M=0, profile `e18-t1-base`, no Service TCP ports, and no VMM
population. The raw completion records `TapAttachBridge`, `set-master`,
netlink `-54`, `measurement_ok=false`, `cleanup_ok=true`, and no cleanup errors.
There are no density sample rows, independent full-population files, or complete
receipt. The before inventory contains exactly seven links, including
`ovd-gbr0`; none has a master or a bridge-port identity. The two normalized
foreign inventories are byte-equal. Normalization is explicitly distinguished
from raw inventory equality.

All nine SHA256 values in the new record were independently recalculated and
match. The synchronized source manifest has 891 entries, all matching current
local files. This proves correspondence with the synchronized source tree; the
executable qualification has the separate limitation in finding F1.

The earlier BASE and PORT4 attempt directories are distinct evidence. Both have
dirty-source digest `f2051cb891547eacdd3622f981f7aa473602c4c7c9ccb99303da3a7f262be951`
and endpoint-deletion/cleanup failures. The new attempt has digest
`28efbbcf98d012531d7eea3d689553ab35e98931d6362c7267c94bac6858f092` and successful
cleanup. Three source-manifest entries differ between those generations: the
benchmark, `guest_tcx.rs`, and its integration test. They must not be conflated.
Their identical `executable_sha256` fields do not establish identical benchmark
executables; F1 reproduces the actual cause of that unusable fingerprint.

| Preserved new-attempt file | Independently verified SHA256 |
|---|---|
| `method.json` | `2a2688bfae29c4cbe55c2f3e91406fa3392bfce868545e008aed32677845a664` |
| `samples.jsonl` | `cea34dc4faafb1775cabce357e0ddcb0afe4e625cca726106d2dd2f901ce320f` |
| `source-files.sha256` | `d6b1f16da614971e665f99165837ea2cb8beda8004b899e341bcace84283e6ca` |
| `prior-shared-state.json` | `8138313398bb5dd4fa2e09716a5da1cdbb5b237ccde874370631c6eba490d6c6` |
| `observations.redb` | `8e5d334372c040d3acd4ef8475dc3f71dea28e4a2ed60fa65cbf4d2719f9ccae` |
| `foreign-before.json` | `361c090206ecb55b56df7c5de54ec50b1ee7ab2ca6200cce09158962bdcd3232` |
| `foreign-after.json` | `361c090206ecb55b56df7c5de54ec50b1ee7ab2ca6200cce09158962bdcd3232` |
| `foreign-before-raw.json` | `361ca2035e5e37a633e04bc3f6c52d35e0ed42253e4ba4ac6c72cbe0447aea6e` |
| `foreign-after-raw.json` | `3c3c06d1367fc539e1e01a9e43b3b15e4af398d9e21d6b3f3c5041d131abc37a` |

## Path, typed cause, and evidence limits

The source path cited by the record matches the current synchronized source:

| Boundary | Evidence |
|---|---|
| Real owner construction | `guest_network.rs:119–120` constructs `HostSharedGuestNetworkOwner`; `:2516` is a private alias for that same concrete owner. |
| Population entry | `netns_density_benchmark.rs:1288–1295` performs attachment 0, warmup quiescence/restore, then serial ascending attachment calls. Successful attachments are retained until cleanup. |
| Allocation effects | `netns_density_benchmark.rs:775–801` obtains the real pool plan, retains it before effects, awaits owner provision, worker intercept installation, and owner activation. |
| Bridge effect | `guest_network.rs:3475–3476`, `:3545–3546`, and `:2132–2137` hold the owner lifecycle lock and await real `Client::set_link_master`. |
| Typed representation | `client.rs:584–592` preserves kernel failure as `NetlinkError::Link` with `set-master`; the owner maps it to `GuestNetworkError::Netlink` with `TapAttachBridge`. The raw completion renders that existing typed cause. No new public error representation is selected. |
| Failure and cleanup | `guest_network.rs:3793–3808` awaits rollback and returns the primary error when it succeeds. Benchmark `:1309–1349` awaits cleanup, restores/comparisons the complement, records failure, and publishes a complete receipt only on success. |

The bounded native attempt is an actual provisioning failure rather than an
imagined cancellation schedule. No new control-plane defect or architecture
finding is inferred from timing, crash/restart, or a test-only abort.

The reviewer independently opened the exact upstream Linux v7.0 sources:
[bridge constants](https://raw.githubusercontent.com/torvalds/linux/v7.0/net/bridge/br_private.h),
[port allocation](https://raw.githubusercontent.com/torvalds/linux/v7.0/net/bridge/br_if.c),
and [errno values](https://raw.githubusercontent.com/torvalds/linux/v7.0/include/uapi/asm-generic/errno.h).
They support 1,024 slots, reservation of slot zero, and propagation of `EXFULL`
from `find_portno` through `new_nbp` and `br_add_if`; `EXFULL` is 54. The review
does not attest the downstream Ubuntu kernel build source.

The 1,023-completed/1,024th-refused ordinal is correctly labelled an inference
from that upstream algorithm, the initially empty bridge, and the serial source
path. The attempt supplies neither an at-failure port dump nor a failing index.
That ordinal must remain conditional on source/path correspondence and the
upstream interpretation; it is not a direct count. The raw provisioning refusal
still proves that this required T1 attempt did not complete, and warrants
preserving the pending execution gate irrespective of F1.

The separate absent-endpoint correction is not approved by this review. Its
native RED is retained in the bounded harness report, and its PASS is reported
in the completed crafter handoff supplied to this review. This review did not
rerun that regression or independently certify its implementation. No E18
hold-time, call-bound, fit, exposure, or capacity conclusion follows from it.

## Dependency and status audit

| Dependency | Disposition and review assessment |
|---|---|
| ADR-0114 sole bridge + T1 population | Correctly pending in part. Two-guest evidence and independent owner/activation/security contracts are preserved. |
| ADR-0117 placeholder | Correctly preserves 16,384 as a fixed placeholder, without inventing a density promise or completion target. Reproducible T1 population feasibility is pending. |
| Feature-delta CAP-295-A | Both exact N/M populations remain recorded: 16,384/0 and 16,384/65,536. No smaller substitute or connection-capacity claim is introduced. |
| 08-04 / E18 | Stopped before full-population validation and sample 0; no completed receipt or step is claimed. |
| 09-01 | Direct dependency on 08-04 is unchanged. L/Q/W/K, derived rustdoc values, both fit checks, double-loss restatement, and R15 hold-threshold validation remain unavailable. |
| ADR-0124 | Only population-dependent measurements/fit/exposure validation becomes pending. Accepted cadence, full audit/restore, kill scope/order, security, and typed failure meanings remain recorded. |
| 10-05 / later execution | Same exact T1 population prerequisite is pending. Existing dependencies prevent 09-02 and phase-10 advancement; no dependency graph is changed. |
| R15 epoch alternative | No completed T1 hold-time measurement exists; its 100 ms trigger has not run. The mutex behavior is not falsified by this bridge refusal. |
| Reopened alternatives | Only ADR-0114's one-bridge-per-workload rejection depended on the shared-switch density rationale. Its independent O(N) cost remains recorded. Independent netns structural cost and ADR-0117 address/map/VMM grounds for 32,768/100,000 remain intact. |
| R18/R19 and completed correct steps | Preserved without alteration. No pending-state schedule, source bound, topology, or new API is selected. |
| Overall roadmap | `validation.status=pending`, `approved_at=null`; the prior R19 approval is historical context, not current execution permission. Approval of this record cannot approve a replacement or resume DELIVER. |

## Normative diff and review gates

The three ADRs' pre-existing accepted text is byte-identical. The feature delta's
pre-existing body from the R19 register onward is byte-identical after undoing
only its historical heading label. No normative `zero`, `never`, `must`,
`exactly`, `fail-closed`, `forbidden`, or `required` outcome is removed or
relaxed. No public API, typed semantic error, persisted/wire format, lifecycle,
ownership, timing rule, population, or capacity criterion changes. No user
waiver is inferred.

A structural JSON comparison against HEAD finds only the overall validation
object and `implementation_notes` of 08-04, 09-01, and 10-05 changed. Descriptions,
criteria, populations, verification commands, dependencies, and file scope are
unchanged. Scoped `git diff --check` passes; all seven local links in the new
record resolve.

The five architecture dimensions support the bounded disposition: no new
technology bias or mechanism, preserved ADR rationale with the affected
alternative reopened, complete population-dependent status scope, actual native
feasibility refusal, and prioritization of the disproved prerequisite. The six
roadmap checks are applied to this delta: invocation/test boundaries and
decomposition are unchanged; no implementation code or new criterion coupling
is introduced; the three status notes explain the gate. Existing roadmap-wide
size/style is outside this review's bounded amendment.

## Findings and remediation

### F1 — HIGH / blocking: the claimed executable fingerprint hashes the utility

**Location:** new factual record, Source correlation, lines 68–71 and
Documentary verification lines 197–199; repeated qualification in the current
status annotations must be read with the corrected evidence limitation.

**Claim:** `48893b0fb21436b54619db80486e83ef39dfccaf1aefe83dfa00c02d6146e8c0`
is described as the executed benchmark's SHA256 and part of qualification of
its compiled source.

**Proven path:** the actual support entry `run` calls `source_and_host` at
`netns_density_benchmark.rs:1244`; that method computes `executable_sha256` at
`:406` using `command("sha256sum", &["/proc/self/exe"])`. The helpers at
`:231–236` spawn a new `sha256sum` process and await its output. Linux
[proc documentation](https://docs.kernel.org/filesystems/proc.html) defines
`self` as the process reading proc and `exe` as that process's executable.
The child therefore hashes its own utility executable, not the benchmark.
No scheduling, shutdown, cancellation, or retry condition is required; this
metadata path executes before each attempted population.

**Independent reproduction:** an exact read-only subprocess invocation on the
already running Lima Linux returns the SHA256 of `/usr/bin/sha256sum`, and
differs from the invoking Python executable's SHA256. The evidence is retained
in [the read-only process-identity receipt](../../../../.context/review-e18-executable-digest-readonly.txt).
That receipt is a mechanism reproduction, not native E18 population evidence.
It does not attest the native utility's exact bytes or recover the benchmark's
missing digest. The same raw field across three source generations is therefore
not evidence of stale benchmark linkage or a dynamic-library mismatch.

**Impact:** the raw hash value is preserved evidence but cannot prove benchmark
identity. The 891 matching manifest entries independently prove synchronized
file contents; they are collected at runtime and do not alone attest the compiled
executable or loaded dependency bytes. The current record overstates that
qualification. This is a factual record finding, not authorization to change
the harness or run another population.

**Required bounded remediation:** keep every raw file unchanged; identify the
existing raw field as produced by the utility-hash mechanism and state that a
benchmark executable digest is unavailable in this receipt. Distinguish verified
synchronized source correspondence/current path audit from compiled artifact
attestation. If an independently preserved build/execution receipt establishes
additional correspondence, cite its exact evidence and limit instead of
assuming it. Preserve the native refusal, upstream inference limits, unchanged
contracts, and pending execution gate. No production/support fix or native
population rerun is required or approved by this finding.

**Disposition:** OPEN; bounded documentary remediation and re-review required.

## Iteration 1 verdict

**CHANGES_REQUESTED.** The native population failure, selective dependent
invalidation, reopened alternative, unchanged normative outcomes, and
non-executable roadmap disposition are supported. One proven executable
qualification overstatement prevents approving the factual record as written.
The exact replacement design and independent roadmap validation remain pending
even after F1 is closed. This review grants no DELIVER resumption permission.

## Iteration 2 — independent documentary remediation review

### Metadata and scope

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261004_e18_native_falsification` |
| Date | 2026-10-04 |
| Role | Fresh isolated replacement solution architecture reviewer, same bounded record |
| Model | GPT 6.1 Sol, extra-high (`xhigh`) reasoning, as explicitly requested |
| Iteration | 2 |
| Verdict | **APPROVED — factual record and status reconciliation only** |
| F1 disposition | **CLOSED — documentary remediation verified** |
| Remaining documentary findings | None |
| Roadmap disposition | **PENDING / NON-EXECUTABLE**; `approved_at=null` |
| Replacement architecture or population | None reviewed, chosen, or approved |

This reviewer first read the complete iteration 1 artifact, then reloaded the
same reviewer definition and its two review skills. The explicit bounded scope,
model, and native Markdown requirements govern this re-review. The original
iteration 1 evidence, finding, OPEN disposition, and CHANGES_REQUESTED verdict
above remain intact as historical records; the disposition below supersedes F1
for the reviewed documentary correction.

The six documentary scope files were reviewed for F1's corrected meaning and
preserved status/contract boundaries. This iteration independently recalculated
the preserved raw-file and source-manifest hashes. It did not reread production
source for a new path review, change support or production code, re-execute the
process-identity reproduction, build, run a population, mutate kernel objects,
derive bounds, stage, or commit. The prior independent path audit and read-only
mechanism reproduction remain the evidence for the already proven F1 cause.

### F1 remediation and evidence

| Corrected claim | Current evidence | Assessment |
|---|---|---|
| Meaning of the raw executable field | Native record, *Source correlation*, lines 70–84, retains the exact `48893b0fb21436b54619db80486e83ef39dfccaf1aefe83dfa00c02d6146e8c0` value and names the `sha256sum /proc/self/exe` subprocess mechanism. | The field is explicitly a utility fingerprint; no benchmark executable digest is claimed to be available. |
| Extent of the source manifest | Native record lines 87–93 and *Documentary verification*, lines 224–227, separate synchronized file correspondence from compiled benchmark and loaded dependency identity. | All 891 current local entries match. Their hashes establish source-file contents only; neither the executable nor loaded dependency bytes are attested. |
| Prior attempts with identical raw fields | Native record lines 79–85 and the preserved read-only Lima receipt. | Identical fields do not establish identical benchmark binaries, stale linkage, or a loaded-dependency mismatch. The Lima reproduction neither hashes the native utility's exact bytes nor recovers the missing benchmark digest. |
| Canonical feature annotation | Feature delta's 2026-10-04 register, lines 58–65. | The utility-hash cause, 891-entry source correspondence, unavailable benchmark digest, and bounded evidence link are explicit. |
| Capture-step note | Roadmap 08-04 `implementation_notes`, line 1403. | The same source/compiled distinction and unavailable benchmark digest are explicit. The native/lease qualification wording elsewhere does not assert compiled-artifact attestation and is read with this recorded limitation. |
| Ordinal and kernel interpretation | Native record, *Inference, not direct inventory*, lines 147–157. | The 1,023-completed/1,024th-refused ordinal remains conditional on synchronized source/path correspondence and upstream interpretation. It is not a direct count or a downstream build-source attestation; the absent compiled benchmark digest is now also explicit. |

**Disposition: F1 CLOSED.** The required remediation is complete in the native
record, feature-delta annotation, and existing 08-04 note. It corrects the
qualification overstatement without editing raw evidence or presenting a new
execution result. No remaining scoped claim treats the recorded raw field or
the source manifest as a benchmark executable/dependency attestation. The
unavailable compiled identity is an explicit evidence limit, not a recovered
fact or an authorization to patch the harness or rerun the population.

### Independent verification and preserved gates

- The attempt still contains exactly nine files. All nine SHA256 values equal
  those independently recorded in iteration 1; `method.json` retains the exact
  raw executable field. Every one of the 891 manifest entries matches its
  current local file; no manifest entry names a compiled artifact or `target/`
  output.
- The failed completion still records `measurement_ok=false`,
  `cleanup_ok=true`, and `cleanup_errors=[]`. The normalized foreign inventories
  are byte-equal. No successful E18 receipt, full-population output, density
  sample, hold-time value, runtime bound, fit result, or threshold verdict is
  introduced by the documentary correction.
- Against HEAD, the three ADRs' pre-existing accepted text remains byte-identical.
  The feature delta's preserved R19 register and subsequent normative body
  remain byte-identical after its historical heading annotation. No required
  population, public interface, typed semantic error, lifecycle/security outcome,
  or normative prohibition is weakened.
- The roadmap's only changed fields against HEAD remain overall `validation`
  and the existing `implementation_notes` for 08-04, 09-01, and 10-05. Criteria,
  exact N/M profiles, verification commands, step dependencies, and implementation
  scope are unchanged. `validation.status` is `pending` and `approved_at` is
  `null`; the prior R19 approval is historical context.
- All nine local links in the current native record resolve. Scoped
  `git diff --check` passes. This reviewer writes only the appended review;
  the six reviewed documents, raw evidence, source/test/harness work, AGENTS,
  DES history, and index are preserved.

For this bounded remediation, the five architecture dimensions and six roadmap
checks retain iteration 1's assessment: F1 adds no architecture mechanism,
population substitute, API, acceptance criterion, dependency, or test boundary.
The native falsification gate, selectively pending dependent validation, reopened
ADR-0114 alternative, independent rejection grounds, and completed R18/R19 and
correct-step evidence remain recorded. The separate uncommitted endpoint
correction remains outside this review's implementation approval.

### Iteration 2 verdict

**APPROVED — factual record and status reconciliation only.** F1 is closed and
there are no remaining documentary findings within this bounded review. The
native provisioning refusal remains a failed attempt; its recorded evidence
does not provide a benchmark executable digest or qualifying E18 measurements.

This approval does **not** approve the roadmap, a replacement architecture or
population, source bounds, implementation work, or DELIVER resumption. The
affected DESIGN and roadmap remain **PENDING / NON-EXECUTABLE**. An exact
replacement DESIGN, the required explicit user decision/approval, independent
DESIGN review, and affected-roadmap validation are still required before
execution resumes.
