# Adversarial review — step 02-02

- **Feature:** `netns-density-295`
- **Step:** `02-02` — shared IP intercept constant-rule replacement and rollback
- **Reviewer:** isolated `nw-software-crafter-reviewer` replacement
- **Review ID:** `code_rev_20260921_171500_iteration_1`
- **Iteration:** 1
- **Reviewed commit:** `797391432fb001cae1d2ca0a0ad7de70986310fa`
- **Parent:** `f1071c9d1b8349ca1eea2c1917bf7291f1fb37ad`
- **Subject:** `feat(mtls-intercept): implement shared IP rule replacement and rollback - step 02-02`
- **Trailer:** `Step-Id: 02-02`
- **Final verdict:** **CHANGES_REQUIRED**

## Executive summary

The exact five-method `MtlsIntercept` shape and the four source-honest rollback
error variants are present, and the three source-local rollback tests pass.
The production host adapter is not approvable: a real Lima-root invocation
fails while creating the shared program because the three set schemas use the
wrong nft ABI type/length, and the failed create leaves a partial table/sets/
chains behind.  Existing-prior replacement also compares kernel rule handles
that are intentionally absent from `InterceptPostcondition`, so target refresh
cannot reach the atomic replace path.  The commit additionally exposes an
unapproved public low-level `nft::ip` bundle, returns a no-op shared guard, and
does not activate the required real-kernel S-ND295-14..19 evidence or a
complete bounded-change Contract Shape proof.

## Iteration history

| Iteration | Commit | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `797391432fb001cae1d2ca0a0ad7de70986310fa` | **CHANGES_REQUIRED** | 6 blockers | Return to the original step-02-02 crafter; re-review this step only after remediation. |

## Authority and scope

The review used the approved step `02-02` in `deliver/roadmap.json`, the exact
PORT-295-C/C-295-C, S2-F01, and ERR-295-A sections of `feature-delta.md`, the
DISTILL S-ND295-14..19 bodies, `distill/red-classification.md`, the accepted
architecture brief, and ADR-0114, ADR-0120, ADR-0122, ADR-0124, and ADR-0125.
The reviewed production caller paths include `HostMtlsIntercept`,
`RealSharedInterceptProgramIo`, `overdrive_netlink::nft::ip`, and the currently
scaffolded `MtlsInterceptWorker::converge_shared_owner`.  No production,
test, or configuration file outside the required review artifact was changed.

## Strengths

- `MtlsIntercept` has exactly the accepted five methods at
  `crates/overdrive-worker/src/mtls_intercept_port.rs:106-254`; no second public
  shared operation was added there.
- `observe_shared` is routed through the real `RealSharedInterceptProgramIo`
  for `HostMtlsIntercept::new()` rather than a test-only constructor.
- The source-local tests cover all four named rollback dispositions, including
  `prior: None`, both rollback operation tags, source-less semantic outcomes,
  idempotent adoption, and the wrong-target runtime branch.
- The commit retains Marcus as author, has exactly one
  `Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 02-02`,
  and changes exactly the two reported production files (971 insertions,
  43 deletions).

## Contract Shape Compliance — iteration 1

**Overall: FAIL.**

| Check | Status | Evidence |
|---|---|---|
| Declaration present | PASS mechanically | The three live source tests at `mtls_intercept_port.rs:619`, `:766`, and `:833` each contain the exact `/// CONTRACT_SHAPE: bounded-change.` line. |
| Banned technical test names | PASS | The three names do not match the repository's banned result/call-count regex. |
| Bounded-change declared delta | **FAIL** | `ScriptedIo` at `:561-603` only queues observations/results and records coarse call kinds; it does not snapshot or mutate the complete eight-rule/three-set/foreign/dynamic-element universe. |
| Complement equality | **FAIL** | The tests assert enum fields and call vectors, but never assert the pre/post program, foreign complement, set complement, listener/EXEC state, or guard-drop state. |
| Driving boundary | **FAIL for S-ND295-19 evidence** | The runtime test calls private `require_shared_program_at_runtime` at `:395-406`; global caller search finds no production caller. `MtlsInterceptWorker::converge_shared_owner` remains a scaffold. |

The private seam is an approved test boundary, but it does not make a queued
result table equivalent to a state-delta proof. The current tests can pass if a
replacement implementation ignores the `expected_current`/`desired` values or
mutates hidden state, because the scripted double never models that state.

## Mechanical evidence — iteration 1

### DES, commit, and scope

| Gate | Result |
|---|---|
| DES RED | PASS — `2026-09-21T13:59:48Z` |
| DES GREEN | PASS — `2026-09-21T14:32:05Z` |
| DES COMMIT | PASS — `2026-09-21T14:33:01Z` |
| Author/trailers/stat | PASS — Marcus, exact Codex trailer and Step-Id, 2 files / 971 additions / 43 deletions |
| `git diff --check` | PASS |
| Affected-package `cargo check --all-targets --features integration-tests` in Lima | PASS |
| Affected library clippy with `-D warnings` | PASS |
| Affected all-target clippy with `-D warnings` | **BLOCKED by pre-existing baseline** — `crates/overdrive-netlink/src/nft.rs:4838` `eprintln!`; no changed-code warning was reported |
| `cargo nextest run -p overdrive-netlink --lib` in Lima | PASS — 56/56 |
| `cargo nextest run -p overdrive-worker --lib -E 'test(shared_program_rollback_acceptance)'` in Lima | PASS — 3/3 |
| Roadmap selector `--test acceptance -E 'test(shared_program_rollback_acceptance)'` | **FAIL** — 0 tests discovered, 99 skipped, nextest exits “no tests to run”; the module is in a library source file, not the acceptance binary |
| Required Tier-3 normalized read-back body | **NOT PRESENT / NOT RUN** |
| Native metal | **UNAVAILABLE** — `OVERDRIVE_METAL_TARGET` is unset |
| Mutation testing | NOT RUN, correctly deferred |

### Real-kernel reproduction

From a temporary untracked harness (removed after the run), Lima root called
the public production netlink adapter after cleaning `table ip overdrive-mtls`:

```text
replace_atomically(None, Some(&SharedProgram::expected(...)))
  -> Err(Nft { op: "atomic-rule-transaction", source: ENODATA })
```

The subsequent real `nft -a list table ip overdrive-mtls` showed the table,
both chains, and all three sets but no rules.  The sets were rendered as
`type verdict`.  The kernel/netlink trace for the valid nft spellings reports
`key_type=7, key_len=4` for `ipv4_addr` and `key_type=461 (0x1cd), key_len=8`
for `ipv4_addr . inet_service`; the commit sends `key_type=1`, `key_len=4`,
and `key_len=6` at `nft.rs:2964-2966`.  This is a real host-adapter failure,
not a scripted-test or metal-availability inference.

The accepted environment matrix assigns S-ND295-14..19 to Lima root, so the
missing native-metal target is not independently fatal for this nft-only lane.
A repaired Lima VM can validly run the same real netlink/nft body.  It cannot,
however, turn the three source-local tests into Tier-3 evidence, and no pass is
claimed here.

## Blocking findings — iteration 1

### D1 — Wrong nft set schemas and non-atomic first-boot create

- **Severity:** Blocker
- **Dimension:** PORT-295-C exact set identity, atomic replacement, and rollback
- **Locations:** `crates/overdrive-netlink/src/nft.rs:2964-2968`, `:3359-3421`

`SET_KEY_TYPE` is hard-coded to `1` with a `NFT_DATA_VALUE` comment, which the
real kernel treats as the verdict datatype, not `ipv4_addr`.  The destination
set is declared with length `6`, while the nft ABI's aligned concatenation is
`ipv4_addr . inet_service` with length `8` and its own datatype code.  The
real Lima reproduction above fails when the first rule lookup is committed.

Even independently of that ABI error, `create()` sends table, each chain, and
each set in separate transactions (`:3359-3401`) and only then sends the rules
in an atomic rule transaction (`:3403-3421`).  When that rule transaction is
rejected, the caller reports `NftSharedReplaceFailed` while the prior `None`
state has become a partial table/chain/set inventory.  That violates the
explicit “batch rejected, prior state intact” disposition and the
atomic-create requirement.

**Required disposition:** use the exact accepted IPv4 and concatenated-set ABI,
and make first-boot creation one atomic effect or perform a source-honest
compensating cleanup that restores exact absence before returning the replace
failure.  Add the real Lima read-back body; do not credit the scripted table.

### D2 — Existing-prior target replacement compares non-identity kernel handles

- **Severity:** Blocker
- **Dimension:** S2-F01 target replacement and exact prior equality
- **Locations:** `nft.rs:2985-3001`, `:3063-3084`, `:3315-3319`, `:3475-3510`; `mtls_intercept_port.rs:424-432`

`Rule` includes `handle: Option<u64>` (`:2985-2991`).  A live dump stores
`Some(rule.handle)` (`:3315-3319`), but `SharedProgram::from_components`, which
reconstructs the worker's `InterceptPostcondition`, stores `handle: None`
(`:3072`, `:3082`).  `ip::replace_atomically` compares the complete derived
`SharedProgram` (`current.as_ref() != expected_current` at `:3480-3485`), so
every present prior reconstructed from `observe_shared` differs solely because
the kernel handle is absent from the postcondition.  `converge_shared` therefore
cannot replace old F/C target ports; it returns a source-bearing replacement
failure before the atomic `Replace` mutations.

The call path is concrete: `HostMtlsIntercept::converge_shared` at
`mtls_intercept_port.rs:499-515` → `replace_observed_shared_program` →
`RealSharedInterceptProgramIo` → `replace_shared_ip_program` →
`nft::ip::replace_atomically`.  The current worker owner caller is still a
later-step scaffold, but this is a defect in the public adapter operation the
step delivers; the real valid-prior fixture is currently blocked earlier by
D1.  The equality must compare normalized semantic components while retaining
live handles only in the private adapter state.

### D3 — Unapproved public low-level API expands the accepted shape

- **Severity:** Blocker
- **Dimension:** Design/API compliance
- **Locations:** `nft.rs:2952-3005`, `:3046-3095`, `:3470-3513`, and the new
  `AtomicRuleMutation::Append/Replace` variants at `:324-350`

The accepted C-295-C contract names one module-private
`SharedInterceptProgramIo` seam and explicitly rejects a public low-level
parameter bundle.  This commit adds a public `nft::ip` module, public
`SharedProgram`, public constructors/projectors, and public observe/replace
functions.  It also adds two variants to the existing public
`AtomicRuleMutation` enum.  The worker now crosses the crate boundary through
that invented public bundle rather than the accepted private seam.  The
five-method `MtlsIntercept` trait itself remains exact, but that does not
authorize an additional public netlink contract.

**Required disposition:** keep the codec/identity/transaction machinery
private or use only an already-approved semantic boundary; remove the new
public variants/types/functions.  Do not add a compatibility method, public
family parameter, or second owner to make the implementation compile.

### D4 — Returned node guard owns no shared state and cannot clean up

- **Severity:** Blocker
- **Dimension:** Shared-object ownership and failed-startup complement
- **Locations:** `mtls_intercept_port.rs:63-72`, `:327-355`

`SharedInterceptGuard` is an empty marker with no `Drop` implementation or
private identity/cleanup state.  Both successful fresh creation and successful
replacement return this marker.  Dropping it therefore cannot delete the
shared table/sets/rules created by an unpublished startup attempt, despite
C-295-C requiring that exact failed-startup cleanup; a later published-owner
relinquish path also has no object to operate on.  The source tests drop the
marker but never inspect kernel state, so this defect is invisible to them.

**Required disposition:** implement the already-approved private guard
ownership/relinquish behavior and prove that failed unpublished startup deletes
only the owned shared universe while published shutdown retains the accepted
constant empty program.  Keep the public `InterceptGuard` shape unchanged.

### D5 — Observation accepts non-canonical identity and is not a complete dump

- **Severity:** Blocker
- **Dimension:** Complete/nonmutating observation, malformed/foreign refusal,
  normalized-rule identity
- **Locations:** `nft.rs:3206-3210`, `:3212-3349`, `:3046-3084`

`collect()` checks rule count, numeric userdata suffix, order, and set/chain
schema, but it never compares each observed normalized expression against the
accepted eight-rule shape while allowing only the two old target-port fields
to vary.  `rule_index()` accepts any parseable numeric suffix, so non-canonical
userdata such as a leading-zero index is accepted as owned.  `from_components()`
also checks only vector shape and userdata, not expression semantics.  A
well-formed rule with the right count/tag family but the wrong match, lookup,
verdict, userdata spelling, or target fields can therefore be returned as an
owned prior instead of a typed conflict.  The bridge-family foreign-table check
also runs only when the IPv4 table is absent (`:3213-3224`).

The independent dumps are not generation-bracketed, so table/chain/set/rule
reads can be assembled from different kernel generations rather than one
complete observation.  This undermines the caller's prior-equality check and
the “preserve every other normalized expression/foreign object” guarantee.

**Required disposition:** validate the canonical normalized eight-rule
projection and exact ownership userdata, reject foreign/partial/duplicate/
malformed identity before mutation, and make the complete observation
consistent with the accepted read-back/concurrency contract.  Preserve old
listener target ports as the only allowed prior variation.

### D6 — Required real production/Tier-3 evidence and bounded-change proof are absent

- **Severity:** Blocker
- **Dimension:** External validity, test honesty, and acceptance coverage
- **Locations:** `mtls_intercept_port.rs:543-853`; absent changes in
  `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs`; the
  roadmap S-ND295-14..19 Tier-3 criterion

The only activated bodies are three source-local tests over `ScriptedIo`.
They do not drive the public `MtlsIntercept::converge_shared`/`observe_shared`
path, do not exercise real `HostMtlsIntercept::new()`, and do not cover the
required malformed/foreign/duplicate/read-back/set-complement cases.  The
runtime no-rewrite test calls the private `require_shared_program_at_runtime`
helper directly; global caller search finds no production caller, while
`MtlsInterceptWorker::converge_shared_owner` is still a RED scaffold.  The
scripted `replace_atomically` ignores both identity arguments and has no
before/after state, so its call-kind assertions cannot prove exact prior
restoration or foreign preservation.

The required real-kernel normalized read-back bodies are not in the commit (or
the current test tree), and the roadmap's acceptance-binary selector discovers
zero tests.  The real Lima reproduction in D1 fails before any acceptance
assertion.  Thus a green source-local suite is not external validity or
Tier-3 evidence.

**Required disposition:** activate the accepted S-ND295-14..19 real host-adapter
bodies on the valid Lima-root lane, enter through the real production
composition/caller, and assert the complete bounded-change universe and
complement (three sets, eight rules, normalized identity, foreign objects,
rollback target including absence, and guard cleanup).  Keep the source-local
seam tests as inner-loop evidence; do not replace the production body with a
test-only caller or infer a pass from unavailable metal.

## External validity

**FAIL.** The `MtlsIntercept` trait shape is present, but no live production
shared-owner caller reaches these new methods in this commit. The only active
shared-program exercise is the private scripted helper, and the required
real-kernel acceptance body is absent. The existing allocation path still uses
the pre-existing install methods; this does not prove the new node-global
program.

## Verification — iteration 1

| Verification | Result |
|---|---|
| DES RED/GREEN/COMMIT | PASS mechanically and chronological |
| Commit author/trailers/stat | PASS |
| `git diff --check` | PASS |
| Lima affected-package check | PASS |
| Lima netlink lib tests | PASS — 56/56, but no new shared-IP integration coverage |
| Lima worker shared-program source tests | PASS — 3/3 scripted tests |
| Exact roadmap `--test acceptance` selector | FAIL — no tests to run |
| Lima real host-adapter create/read-back | FAIL — `ENODATA`, partial table/sets/chains remain |
| Lima affected library clippy | PASS |
| Lima all-target clippy | BLOCKED by pre-existing `nft.rs:4838` `eprintln!`; no changed-code warning |
| Native metal Tier-3 | NOT AVAILABLE; no pass inferred |
| Mutation testing | NOT RUN, correctly deferred |

## Quality gates — iteration 1

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance body | FAIL substantively | Only private scripted bodies are active; the roadmap acceptance selector finds none. |
| G2 — valid RED | PASS mechanically | DES RED is present and reported PASS. |
| G3 — assertion failure protects behavior | **FAIL** | The scripted double ignores identity arguments and has no state/complement oracle. |
| G4 — no unsanctioned domain mocks | PASS for the approved private seam | The seam itself is design-approved; it is not real-adapter evidence. |
| G5 — contract language/shape | **FAIL** | Bounded-change universe/complement is not declared in assertions. |
| G6 — all required evidence green | **FAIL** | Real Lima host-adapter create fails; Tier-3 body is absent. |
| G7 — green before commit | PASS mechanically | DES GREEN precedes COMMIT. |
| G8 — test budget | PASS | Six mapped behaviors, budget 12; three source test methods. |
| G9 — no prohibited test weakening | PASS | The commit only removes the three pending `#[ignore]` markers; no assertion was weakened or deleted. |

## RPP scan

- **Levels scanned:** L1-L4.
- **Cascade stopped at:** L4 because the blockers are contract, ownership,
  adapter, and evidence-boundary defects rather than cleanup-only readability
  issues.
- The new public low-level bundle is speculative API surface under the accepted
  private-seam design; the empty guard and split create path are correctness
  issues, not style recommendations.

## Remediation disposition — iteration 1

| Finding | Status | Owner/action |
|---|---|---|
| D1 — set ABI and atomic first-boot create | OPEN | Original step-02-02 crafter: repair exact nft schemas and prior-absence atomicity/cleanup; add real Lima read-back. |
| D2 — handle-insensitive prior identity | OPEN | Original step-02-02 crafter: keep kernel handles private and compare only normalized semantic identity before replacement. |
| D3 — public low-level API expansion | OPEN | Original step-02-02 crafter: return to the approved private codec/seam; no new public type/variant/function. |
| D4 — no-op shared guard | OPEN | Original step-02-02 crafter: implement accepted private ownership/drop/relinquish behavior and complement proof. |
| D5 — incomplete identity observation | OPEN | Original step-02-02 crafter: canonical rule validation, foreign/malformed refusal, and complete consistent read-back. |
| D6 — missing real caller/Tier-3 and Contract Shape proof | OPEN | Original step-02-02 crafter: activate the accepted real Lima body, use production composition, and assert complete bounded-change state/complements. |

No finding is waived or deferred. Under this repository's no-iteration-cap
rule, the original crafter must remediate this same step and this reviewer
must re-review until the verdict is `APPROVED`.

## Iteration-1 final verdict

**CHANGES_REQUIRED**

Step 02-02 must not advance. The source-local rollback table is useful inner
loop evidence, but the real adapter currently fails its first-boot ABI path,
cannot retarget an existing prior identity, leaks the returned shared guard's
ownership, exposes an unapproved public surface, and has no honest Tier-3 /
Contract Shape proof. Native-metal unavailability is not used as an excuse for
these defects; the accepted Lima lane is sufficient for this step once the
production body is repaired and actually exercised.

## Iteration 2 — D15 remediation re-review

### Metadata

- **Reviewer:** fresh isolated DELIVER reviewer replacement
- **Review ID:** `code_rev_20260921_223000_iteration_2`
- **Step:** `02-02` — shared IP intercept constant-rule replacement and rollback
- **Feature:** `netns-density-295`
- **Reviewed cumulative commit:** `f1bd1fa1720768d09e4a12783e88ebbe39d22d23`
- **Commit parent:** `a460cf7dce5370b4dffd4e7aab8b5c7041fd89cc`
- **Prior review:** iteration 1 in this artifact, commit
  `797391432fb001cae1d2ca0a0ad7de70986310fa`
- **Final verdict:** **APPROVED**

### Scope and authority

This re-review covers the cumulative implementation through the reviewed
commit, the six iteration-1 findings, the approved D-295-DISTILL-15 contract,
the final D15/S19 translation review, and the step-02-02 roadmap criteria. It
does not credit the pending 02-03 published-worker layer or the pending 03-03
control-plane cadence/fail-stop layer. No production code, tests, or commits
were changed by this reviewer; this append is the sole tracked write.

### Strengths

- The exact doc-hidden `SharedIpInterceptIdentity` API is present at
  `crates/overdrive-netlink/src/nft.rs:2913-2970`, with private semantic fields
  and no public family, raw-builder, handle, or generic mutation surface.
- The private observed `Rule` retains kernel handles only inside the netlink
  adapter (`nft.rs:3012-3017`, `:3467-3480`), while `SharedProgram::from_components`
  reconstructs handle-free semantic identity (`:3072-3092`).
- The source-local stateful fixture owns the complete declared universe,
  conditional mutation journal, and staged fault schedule. The three live
  bounded-change bodies cover the full optional-prior/trigger/rollback table,
  refusal/create/retarget/reapply/drop transitions, caller-prior drift, and
  independent zero-port refusal.
- The four real Lima bodies drive `HostMtlsIntercept::new()` and the public
  `MtlsIntercept` methods. They read back the real three sets/eight rules,
  generation snapshots, notifications, dynamic complement, and unrelated
  foreign table; the ambiguous-state body covers all seven accepted refusal
  partitions (`mtls_intercept_install.rs:431-701`).
- The final commit preserves Marcus as author and contains exactly one
  `Co-Authored-By: Codex <codex@openai.com>` and one `Step-Id: 02-02` trailer.

### Iteration-1 finding dispositions

| Finding | Status | Evidence and disposition |
|---|---|---|
| **D1 — wrong set ABI / non-atomic first create** | **CLOSED** | The adapter uses the kernel IPv4 datatype `7`, concatenated datatype `0x1cd`, lengths `4` and `8` (`nft.rs:2987-2995`), and serializes table, chains, sets, and eight rules into one acknowledged nft batch (`:3545-3661`, `:3663-3708`). `None -> Some`, replacement, and `Some -> None` all use the same complete transaction boundary. Lima absence/create/read-back and guard-drop passed. |
| **D2 — handle-dependent prior equality** | **CLOSED** | Semantic equality is made from the handle-free identity only; private handles are attached to observed rules and used only after the semantic comparison succeeds (`nft.rs:3012-3024`, `:3467-3516`, `:3735-3785`). The Lima retarget body passed with exact target-only semantic delta. |
| **D3 — unapproved public low-level API** | **CLOSED** | The rejected `nft::ip` bundle and `AtomicRuleMutation` expansion are absent. The root-level public surface exactly matches D15: three identity methods and two doc-hidden functions (`nft.rs:2913-2970`); `AtomicRuleMutation` remains the pre-step Delete/Insert shape (`nft.rs:308-333`). |
| **D4 — no-op shared guard** | **CLOSED for 02-02** | `SharedInterceptGuard` owns the private I/O handle plus requested identity and performs one conditional unpublished cleanup on Drop (`mtls_intercept_port.rs:63-83`). Stateful and Lima bodies prove stale guards cannot delete a successor or foreign state and that current unpublished Drop restores absence. The published relinquish path remains explicitly deferred to 02-03, as required by D15. |
| **D5 — incomplete/non-canonical observation** | **CLOSED** | Observation is generation-bracketed (`nft.rs:3519-3529`), validates exact chains, set schemas/userdata, empty dynamic sets, foreign children/family, rule count/order/userdata, canonical expression identity, and the two non-zero target ports (`:3274-3333`, `:3376-3516`). Ambiguous real-kernel fixtures refuse without mutation. |
| **D6 — missing production/Tier-3 and bounded-change evidence** | **CLOSED for the step boundary** | The three stateful source-local bodies and four exact Lima selectors are live, separately named, and carry `/// CONTRACT_SHAPE: bounded-change.`. The Lima bodies enter through `HostMtlsIntercept::new()` and the public port; S19-A remains adapter-only. The 02-03 worker prerequisite and 03-03 supervisor body remain pending and are not required for this step's adapter approval. |

No iteration-1 correctness finding remains open. The 02-03 published owner and
03-03 retry/deadline/fail-stop behaviors remain correctly outside this verdict.

### Contract shape and test-boundary review

**PASS.** The three live source-local stateful tests at
`mtls_intercept_port.rs:919`, `:1211`, and `:1381` each declare the exact
bounded-change Contract Shape. The S19-A source-local body at `:1666` and the
four Lima bodies at `mtls_intercept_install.rs:431`, `:493`, `:542`, and `:597`
also carry the declaration. The source-local bodies assert the production host
algorithm through the approved module-private seam; the Lima bodies assert the
real public adapter and kernel state. No test imports or exposes a private
netlink handle, substitutes a fake production caller for Lima, or claims the
pending worker/supervisor layers.

### Behavioral evidence

#### Exact schema, atomicity, and semantic identity

The final code encodes the exact set schemas through
`newset_payload_schema_family` (`nft.rs:947-965`), uses the approved IPv4 and
concatenated datatype identifiers (`:2987-2995`), and sends one mixed-object
batch with per-operation ACK draining (`:3545-3661`). The private `SharedMutation`
enum is not public. `replace()` performs a complete generation-bracketed
conditional observation before selecting create, replacement, or deletion
(`nft.rs:3735-3785`). This closes D1 and D2 without adding a second adapter or
public operation.

#### Canonical refusal and complement preservation

`collect_once()` rejects same-name foreign-family tables, partial/foreign
chains, wrong chain schema, wrong/duplicate/missing set identity, non-empty
dynamic sets, stateful children, wrong rule count/order/userdata, and non-
canonical expression programs before mutation (`nft.rs:3376-3516`). The
integration fixture enumerates foreign family, incomplete program, duplicate
owned rule, unknown userdata, conflicting set schema, foreign child, and
non-empty dynamic set (`mtls_intercept_install.rs:607-701`). Each row compares
the complete target snapshot and foreign sentinel before and after refusal.

#### Rollback algebra and source/state proof

The desired-read failure branch rolls back conditionally from the committed
requested identity (`mtls_intercept_port.rs:359-400`); the semantic-mismatch
branch rolls back from the observed replacement (`:403-444`). Each branch makes
one rollback attempt and one rollback read where specified. The source-local
stateful table asserts both `prior = None` and `prior = Some`, all trigger and
rollback outcomes, exact operation tags, separate desired-read and rollback
sources, source-less semantic outcomes, complete owned-program transitions,
unchanged dynamic/foreign complements, and closed mutation journals
(`mtls_intercept_port.rs:919-1209`).

#### Guard ownership split

Successful fresh/adopt/retarget operations return the private guard only after
the desired identity is read back (`mtls_intercept_port.rs:339-407`). Its
conditional Drop is proven by the source-local fixture and the real Lima
absence/create/reapply and retarget bodies. The stale prior guard refuses to
delete a successor; the current unpublished guard deletes only the complete
owned graph. The published-owner sealed relinquish remains the explicitly
pending 02-03 responsibility, with no public disarm/relinquish method added.

### Mechanical verification

| Check | Result | Evidence |
|---|---|---|
| Reviewed commit scope | PASS | `f1bd1fa1`: four files, 637 additions, 358 deletions; no unrelated production path changed in the remediation commit. |
| Author/trailers | PASS | Marcus author; exactly one Codex trailer and one `Step-Id: 02-02`. |
| `git diff --check` / formatting | PASS | Both pass on the reviewed tree. |
| Lima source-local D15 selectors | PASS | Independently ran all three stateful bodies: 3 passed; independently ran S19-A: 1 passed. |
| Lima `overdrive-netlink` library | PASS | 56 passed, 0 skipped. |
| Lima `overdrive-worker` library | PASS | 62 passed, 12 reasoned ignored. |
| Lima real-adapter selectors | PASS | Four exact public-adapter bodies passed; 47 unrelated integration bodies were skipped by the selector. |
| Lima nextest group assignment | PASS | All four exact integration bodies resolve to `host-kernel-shared` with `max threads = 1`. |
| Lima workspace check | PASS | `cargo check --workspace --all-targets --features integration-tests`. |
| Focused worker clippy | Reported PASS; independent rerun environment-limited | The crafter/DES report records focused `-D warnings` clippy as PASS. A later independent rerun could not write more metadata because the Lima `/tmp` tmpfs was full; no changed-code warning was reported. |
| All-target clippy | Baseline-limited | The recorded all-target run is blocked only by the pre-existing `overdrive-netlink/src/nft.rs:4838` test `eprintln!`; no D15 warning was reported. |
| Mutation testing | NOT RUN | Correctly deferred to the final DELIVER-wave gate. |

Native metal is not required for this nft-only D15 lane; the accepted matrix
assigns these four real-adapter bodies to Lima root. The source-local macOS
attempt was also correctly environment-limited by Linux-only netlink constants,
not used as a production or test finding.

### DES and committed-log disposition

The final cumulative DES audit contains canonical replacement
`RED/GREEN/COMMIT` events at `20:31:43Z`, `20:31:45Z`, and `20:31:45Z`, all
`EXECUTED/PASS`. The committed execution log also preserves the earlier
remediation attempt's historical `FAIL` and the subsequent superseded
`GREEN/COMMIT` records. This is an audit-history/scope note only: it does not
invalidate the canonical replacement cycle or the verified final commit, and
the preserved history is not rewritten by this review.

### New findings

No new reachable behavioral, API-shape, test-honesty, ownership, or scope
finding was identified. The only non-blocking evidence notes are the preserved
DES history and the pre-existing all-target clippy baseline recorded above.

## Iteration-2 final verdict

**APPROVED**

The final D15 implementation closes D1-D6 within the approved 02-02 boundary:
exact nft schemas and atomic complete-object transitions are real-kernel green;
semantic equality is handle-free while private handles remain available for
replacement/deletion; the public surface is exactly the approved doc-hidden
netlink API; unpublished Drop cleanup is distinct from the pending published
relinquish path; observation is generation-bracketed and canonical; the
stateful source-local and Lima evidence layers are paired and complete; and
post-commit read-failure rollback preserves the exact error/source/state
algebra. Step 02-02 may advance to its next orchestrated step.
