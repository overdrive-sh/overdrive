# Product Review — expectation boundary restoration

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Target commit | `9558af759e049564b7149bea6459961979899e96` |
| Review type | Fresh isolated PRODUCT review |
| Reviewer | `nw-product-owner-reviewer` (Eclipse), targeted DISTILL boundary review |
| Review date | 2026-08-29 |
| Iteration | 1 |
| Verdict | **NEEDS_REVISION** |

## Verdict summary

The commit substantially restores the intended one-expectation boundary, but
the sole checked-in operator journey is not runnable honestly as written. Its
Exec Service implicitly receives the repository's known-unreachable production
TCP startup probe, so an interactive `overdrive deploy callee.toml` reaches
`Failed` before the documented journey can deploy the VM caller. The example
also references three materialized artifacts that neither exist in the checkout
nor have a checked-in preparation command. Two additional boundary-integrity
issues remain: S-GTI-01 still contains private wire/confidentiality clauses
despite being described as public-success-only, and the executable pending E07
stub produces a successful narrated run that the harness records as executed.

Approval requires zero blocker, high, or medium findings. This iteration has
one blocker, one high, and two medium findings.

## Finding counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| High | 1 |
| Medium | 2 |
| Low | 0 |

## What is correct

- The active feature catalogue now has exactly one expectation: E07. The E08
  and E09 directories, index rows, roadmap mappings, and active DISTILL
  mappings are removed. References retained only inside historical review
  artifacts are audit history, not active mappings.
- The checked-in bundle contains exactly one `[service]` + `[exec]` callee and
  one `[job]` + `[vm]` caller. The caller uses ordinary plaintext TCP, resolves
  `gti-e07-callee.svc.overdrive.local`, and exits successfully only after a
  byte-exact, byte-distinct reply.
- E07's current prose limits evidence to built-product commands and the public,
  reply-dependent successful result. It explicitly excludes D7 framing,
  normalized nft programs, counters, capture, TLS/kTLS, generation stability,
  and private cleanup from the expectation.
- Boot failure, diagnostics, failed install, pre-READY behavior,
  restart/reclamation, stop/idempotency, sibling preservation, nft/FIB,
  cleanup, and replay remain mapped to Rust acceptance/component/native tests.
- The stale DELIVER roadmap is marked `validation.status = pending` with
  `requires_regeneration = true`; the superseded approval is not presented as
  executable authority.
- Both checked-in Rust helper sources compile as Rust 2024 metadata, the E07
  shell scaffold passes `bash -n`, and `git show --check` reports no whitespace
  errors for the target commit.

## Findings

### B-01 — The sole operator journey's first deploy is driven to `Failed`

**Severity:** Blocker

**Evidence**

- `examples/guest-stack-transparent-mtls-intercept/callee.toml:13-15`
  declares a TCP listener but contains no explicit startup-probe choice.
- `crates/overdrive-core/src/aggregate/workload_spec.rs:1275-1340` specifies
  that a Service with a listener and no explicit startup section receives an
  inferred TCP probe to `0.0.0.0:<listener-port>`.
- `crates/overdrive-cli/src/commands/deploy.rs:613-687` shows that an
  interactive Service deploy streams until `Stable`, `Failed`, or `Stopped`;
  `Failed` is terminal with exit code 1.
- The repository's root-example precedent documents the production behavior
  directly: `examples/quick-bind-service.toml:39-60` says the probe runner does
  not enter the workload network namespace and the inferred TCP probe drives a
  serving Service to `Failed { StartupProbeFailed }` after 60 seconds.
- The new README's exact operator sequence runs the callee deploy first
  (`examples/guest-stack-transparent-mtls-intercept/README.md:19-25`). In a
  terminal, that command therefore fails before the VM caller deploy is
  reached. A non-TTY/detached capture merely turns this into a race against the
  same 60-second failure; it does not make the example honest.

**Product impact**

The only checked-in journey cannot reliably demonstrate its only promised
outcome. This is the central stakeholder contract, so the handoff cannot pass.

**Required remediation**

Make the callee's startup policy explicit and production-reachable. For this
narrow call-success example, the smallest honest correction is:

```toml
[health_check]
startup = []
```

That uses the repository's explicit opt-out contract and makes first Running
become Stable. An explicit exec probe is also acceptable if it is executed in
the workload context and demonstrably passes. Then make the README and E07
runner use the same lane and prove the callee remains available through the
call, rather than relying on non-TTY auto-detach timing.

### H-01 — The checked-in example still depends on phantom preparation

**Severity:** High

**Evidence**

- `callee.toml:6` points to
  `/var/lib/overdrive/examples/guest-stack-transparent-mtls-intercept/callee`.
- `caller.toml:5-8` points to a caller binary inside the guest plus host kernel
  and rootfs paths. None of those three materialized files is checked in.
- `examples/guest-stack-transparent-mtls-intercept/README.md:9-17` says to run
  the journey only "after compiling" and "installing" the helpers, but supplies
  no executable preparation command, no base-kernel/rootfs input contract, and
  no reproducible install procedure.
- The only runner is explicitly pending. It checks that four source/spec files
  exist, narrates a future runner, and performs none of the required
  materialization.
- The established root-example convention is concrete about this failure
  class: `examples/dial-by-name-responder/a.toml:18-34` uses a real interpreter
  plus a checked-in program so the example runs by hand without an unseen
  staging step. Its feature record explicitly rejected a staged binary as a
  phantom path.

**Product impact**

The bundle is an executable design fixture, not yet an operator-runnable
example. An operator following the checked-in README reaches missing command,
kernel, or rootfs paths before exercising the product.

**Required remediation**

Land one checked-in, operator-invocable preparation entry point in the example
bundle. It must accept or discover a documented real base kernel/rootfs, compile
the checked-in helpers without generating source/manifests/specs, clone and
install the guest helper, materialize the exact paths referenced by both specs,
and clean up its own outputs. The E07 runner must call that same entry point so
the product example and verification fixture cannot drift. Where
materialization is avoidable (notably the host Exec Service helper), prefer a
real checked-in program invoked through an existing on-disk interpreter, or
otherwise include the exact compile/install command in the shared preparation
entry point.

### M-01 — The public-success anchor still contains private wire claims

**Severity:** Medium

**Evidence**

- `distill/test-scenarios.md:95-99` says S-GTI-01 states only the
  stakeholder-visible reply and that S-GTI-02 owns D7.
- The actual S-GTI-01 still requires that the connection be protected end to
  end and that no cleartext copy exist on the peer path, including before
  Running (`distill/test-scenarios.md:231-236`). Those are private wire/security
  claims, and S-GTI-02/S-GTI-03 already own them at lines 238-258.
- E07 anchors only an undefined "stakeholder-visible slice" of S-GTI-01, while
  its directory/contract identifier still says
  `E07-guest-first-mesh-dial-born-captured`. The body is success-only, but the
  anchor and identity continue to imply the removed D7 claim.

**Product impact**

The EDD boundary is understandable only by reading the explanatory caveat and
ignoring part of the named anchor. That is fragile: a future evidence author can
reasonably treat the whole anchored scenario or the expectation identifier as
the contract and reintroduce internal D7/wire evidence.

**Required remediation**

Remove S-GTI-01's two private clauses; S-GTI-02 and S-GTI-03 already retain
them in Rust. Make E07 anchor the complete, success-only S-GTI-01. Rename the
E07 slug to a success-only name such as `E07-vm-job-calls-exec-service` and
update the active index, DISTILL, and pending-roadmap mappings, while retaining
the numeric E07 identity if continuity is desired.

### M-02 — The executable pending runner can create narrated "executed" evidence

**Severity:** Medium

**Evidence**

- E07's `runner.sh` is executable (`100755`), checks only file presence, prints
  a future-tense narrative, and exits 0 (`runner.sh:7-19`). Direct execution
  confirms exit 0 without invoking the built product.
- `verification/harness/run-expectation.sh:70-83` sets `EXECUTED=true` solely
  because an executable runner exists. Lines 88-103 then write
  `executed_in_lima: true` and `runner_exit_code: "0"` to the evidence manifest.
  The harness has no pending-stub discriminator.
- `verification/expectations/E07-guest-first-mesh-dial-born-captured/README.md:50-52`
  first says no evidence was captured, then says "deleted historical E07
  evidence" attempted to prove D7. The parent contract also said no evidence
  was captured; the deleted object was a contract, not evidence.

**Product impact**

Running the ordinary harness today creates a commit-pinned manifest that looks
successfully executed even though it contains only narration. The README's
false historical wording further weakens the evidence audit trail.

**Required remediation**

Until the real built-product runner lands, keep the scaffold non-executable (or
remove it); the existing harness then records `executed=false`/`n/a` and leaves
the expectation honestly pending. Do not solve this with another successful
placeholder branch. Replace the historical sentence with the exact fact: no
prior E07 evidence was captured; the superseded E07 contract, not evidence,
contained the internal D7 requirements.

## Boundary assessment

| Required boundary | Result | Evidence |
|---|---|---|
| Exactly one feature expectation | PASS | Active catalogue, index, DISTILL, and pending roadmap map only E07 |
| One checked-in example journey | PASS structurally | One bundle contains one callee Service and one caller Job |
| One VM Job calls one Exec Service | PASS in fixture shape | `caller.toml` names the sole service and port; helpers require an exact reply |
| Built product is the driving surface | PASS in contract | E07 names built `serve`, `deploy`, `workload describe`, and `job stop` |
| Only visible call success is expected | NEEDS_REVISION | E07 body is narrow, but S-GTI-01/slug retain private D7/wire claims (M-01) |
| E08/E09 and private lifecycle/kernel contracts stay Rust-only | PASS | Active mappings are Rust-only; historical review references remain audit history |
| Example is operator-runnable and honest | FAIL | Known startup-probe failure (B-01) and missing preparation/materialized paths (H-01) |
| No generated/phantom fixtures | FAIL | Sources/specs are checked in, but the actual command/kernel/rootfs inputs have no checked-in materialization path (H-01) |
| Verification language cannot masquerade as evidence | FAIL | Executable pending stub exits 0 and is recorded as executed (M-02) |

## Verification performed

- Inspected the complete target commit and all sixteen changed paths.
- Compared active expectation/index/DISTILL mappings with the parent commit and
  searched for remaining E07/E08/E09 references.
- Read the approved DESIGN Q9/D7 decisions, corrected DISTILL scenarios and RED
  classification, feature delta, pending roadmap, verification operational
  rules, E07 files, and root example precedents.
- Confirmed both helper sources compile with `rustc --edition=2024
  --emit=metadata`.
- Confirmed `bash -n` passes for E07 and directly observed that the pending
  runner exits 0 after narration only.
- Confirmed `git show --check 9558af759e049564b7149bea6459961979899e96`
  is clean.
- Mutation testing was not run, as required.

## Final verdict

**NEEDS_REVISION** — remediate B-01, H-01, M-01, and M-02, then repeat this
targeted product review against a fresh immutable commit.

---

## Iteration 2 — Remediation re-review

### Metadata

| Field | Value |
|---|---|
| Target commit | `f064e0566611b1b8c4da7775fc169b929bccbca3` |
| Compared with | Parent `9558af759e049564b7149bea6459961979899e96` and Iteration 1 above |
| Review type | Same-reviewer PRODUCT remediation re-review |
| Review date | 2026-08-29 |
| Verdict | **APPROVED** |

### Verdict summary

All four Iteration 1 findings are closed. The sole checked-in example now has
an explicit no-startup-probe policy, one reproducible preparation entry point,
and one bounded operator entry point. The public scenario and E07 identity now
describe only the reply-dependent call outcome. The executable placeholder
fails closed with exit 75, which the harness records as pending and returns
nonzero rather than presenting as executed evidence.

The active feature boundary contains exactly one expectation, E07. Its bundle
contains exactly one `[service]` + `[exec]` callee and exactly one `[job]` +
`[vm]` caller. No active E08 or E09 catalogue directory, mapping, or roadmap
identity remains; their failure, recovery, lifecycle, kernel, and wire
contracts stay Rust-owned. E07 remains honestly `pending`; this approval is of
the DISTILL/product boundary and runnable example contract, not a claim that
runtime evidence has been captured.

### Finding counts

| Severity | Open | Resolved from Iteration 1 |
|---|---:|---:|
| Blocker | 0 | 1 |
| High | 0 | 1 |
| Medium | 0 | 2 |
| Low | 1 | 0 |

### Iteration 1 remediation dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| B-01 — inferred startup probe fails the callee | **RESOLVED** | `callee.toml:17-21` explicitly declares `startup = []`. `run-example.sh:290-294` waits for the public `Stable` state before deploying the caller, so the journey no longer races the known-unreachable inferred probe. |
| H-01 — phantom preparation | **RESOLVED** | `prepare.sh:66-84` binds checked-in sources/specs to exact materialized paths; lines 168-219 compile static helpers, reflink the qualified rootfs, install the guest caller, stage the kernel/callee, and create the production KEK input. `run-example.sh:263-301` provides one bounded operator entry point using that same preparation contract and the built default-feature binary. |
| M-01 — public anchor retains private wire claims | **RESOLVED** | S-GTI-01 at `distill/test-scenarios.md:230-234` now ends at the byte-distinct reply. S-GTI-02/S-GTI-03 retain D7 and wire claims. The active slug is `E07-vm-job-calls-exec-service` throughout the catalogue, DISTILL map, and pending roadmap. |
| M-02 — successful narrated evidence | **RESOLVED** | The E07 stub at `runner.sh:7-13` performs only the source association check and exits 75. `run-expectation.sh:85-104` maps 75 to `execution_status: pending`; lines 123-140 persist that state, warn against satisfaction, and return nonzero. The E07 README accurately states that no E07 evidence has ever been captured. |

### What is correct in the remediated boundary

- `git ls-tree` contains one feature expectation directory only:
  `verification/expectations/E07-vm-job-calls-exec-service/`. Exact searches
  find no active former E07 slug and no E08/E09 expectation identity outside
  historical review artifacts.
- The example tree contains two checked-in workload specs, two checked-in Rust
  helpers, one preparation script, one operator script, and its README. The
  specs declare one Service/Exec callee and one Job/VM caller; the caller uses
  ordinary service-name TCP and returns zero only after the exact reply.
- The operator README provides the canonical `cargo xtask metal run --`
  invocation, qualified kernel/rootfs inputs, preparation behavior, bounded
  lifecycle, and marker-owned cleanup. It explicitly says the journey proves
  only stakeholder-visible call success.
- The E07 contract restricts product-driving actions to the built
  `serve`/`deploy`/`workload describe`/`job stop` surface. Strict netlink/nft,
  capture/counters, TLS/kTLS, original-destination, generation, and private
  lifecycle guarantees remain assigned to Rust.
- The roadmap remains non-executable: `validation.status` is `pending` and
  `requires_regeneration` is `true`. Its sole EDD identity is the renamed E07.

### L-01 — Two catalogue navigation notes still say every runner uses Lima

**Severity:** Low (non-blocking)

`verification/README.md:110` still describes `run-expectation.sh` as running
the runner in Lima, and `verification/expectations/INDEX.md:181-182` tells an
author to use the Lima-only `od` helper. The governing text immediately above
now correctly supports a declared `native-metal` substrate, and E07's own
contract is unambiguous, so these stale navigation notes do not weaken this
feature boundary. Generalize them to “declared substrate” in a later
documentation cleanup.

### Boundary assessment

| Required boundary | Result | Evidence |
|---|---|---|
| Exactly one feature expectation, E07 | PASS | One active E07 directory/index row/roadmap identity; no active E08/E09 identity |
| One checked-in operator example | PASS | One feature bundle with a documented metal invocation and shared preparation/run entry points |
| One VM Job calls one Exec Service | PASS | `caller.toml` is `[job]+[vm]`; `callee.toml` is `[service]+[exec]`; exact request/reply constants match |
| Operator journey is runnable without phantom fixtures | PASS | `prepare.sh` materializes every unavoidable binary/kernel/rootfs path from checked-in sources and qualified inputs |
| Callee remains available for the call | PASS | Explicit empty startup policy plus bounded public Stable observation |
| Only stakeholder-visible call success is expected | PASS | Complete S-GTI-01 and E07 require the reply-dependent successful result only |
| E08/E09 and internal contracts remain Rust-only | PASS | Active mappings assign D7/wire/failure/recovery/cleanup/replay to Rust and expose no E08/E09 expectations |
| Pending scaffold cannot masquerade as evidence | PASS | Exit 75 becomes pending, produces a nonzero harness result, and cannot be marked satisfied |

### Verification performed

- Read the complete Iteration 1 artifact and inspected every path changed by
  `f064e0566611b1b8c4da7775fc169b929bccbca3` against its parent.
- Confirmed the active catalogue/tree and roadmap contain only the renamed E07
  identity; exact searches found no active former-E07, E08, or E09 expectation.
- Ran `shellcheck` and `bash -n` on both example scripts, the E07 stub, and the
  changed harness; all passed.
- Ran `prepare.sh check-source`; it passed and verified source/spec/materialized
  path association plus the explicit startup opt-out.
- Executed the E07 stub directly and observed exit 75 after pending-only text.
- Compiled both helper sources as Rust 2024 metadata; both passed.
- Parsed the roadmap with `jq`, confirmed pending/regeneration-required state,
  and confirmed `git show --check` is clean for the target commit.
- Native-metal product execution was not run and no E07 evidence was claimed.
  Mutation testing was not run.

### Final verdict

**APPROVED** — Iteration 1's blocker, high, and two medium findings are closed.
The one low-severity catalogue-navigation wording issue is non-blocking and
does not alter the sole-E07 product boundary.

---

## Iteration 3 — Runtime-boundary and harness compatibility re-review

### Metadata

| Field | Value |
|---|---|
| Target commit | `5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Cumulative review range | `9558af759e049564b7149bea6459961979899e96..5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Remediation commits inspected | `f064e0566611b1b8c4da7775fc169b929bccbca3`, `5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Review type | Same-reviewer PRODUCT remediation re-review |
| Review date | 2026-08-29 |
| Verdict | **APPROVED** |

### Verdict summary

All Iteration 1 findings remain closed, and Iteration 2's sole low finding is
resolved. The additional remediation removes three latent ways the runnable
example could have crossed its product boundary: it parses the actual public
Service and Job describe tables separately, gives `serve` a fresh anonymous
session keyring instead of touching an ambient KEK entry, and limits cleanup to
successful public stop operations, the exact verified `serve` process, and
token-owned preparation materialization. It no longer inspects or repairs
product-private processes, run directories, cgroups, namespaces, links, or
capture state.

The active feature contract still contains exactly one expectation, E07, and
one example containing one `[service]` + `[exec]` callee and one `[job]` +
`[vm]` caller. E08 and E09 remain Rust-only concepts rather than expectations.
The harness/E06 compatibility changes are coherent: E06 now declares
`native-metal`, historical evidence is explicitly preserved as historical,
fresh manifests use accurate substrate fields, and every pending, absent, or
failed runner path returns nonzero. E07 remains honestly `pending`; no runtime
evidence or satisfaction claim is introduced.

### Finding counts

| Severity | Open | Resolved this iteration | Previously resolved and still closed |
|---|---:|---:|---:|
| Blocker | 0 | 0 | 1 |
| High | 0 | 0 | 1 |
| Medium | 0 | 0 | 2 |
| Low | 0 | 1 | 0 |

### Prior-finding dispositions

| Finding | Iteration 3 disposition | Evidence |
|---|---|---|
| B-01 — inferred startup probe fails the callee | **STAYS RESOLVED** | `callee.toml:17-22` retains explicit `startup = []`. `run-example.sh:198-215` observes the real public Service table as `Alloc/State=Running` with replicas `1/1` before the caller deploy. |
| H-01 — phantom preparation | **STAYS RESOLVED** | `prepare.sh:197-260` still compiles both checked-in helpers, stages the qualified kernel and reflinked rootfs, installs the guest caller, and creates the exact credential/data paths. `run-example.sh:236-322` remains the single bounded operator entry point using that preparation contract and the built default-feature binary. |
| M-01 — public anchor retains private wire claims | **STAYS RESOLVED** | S-GTI-01 remains the complete reply-only stakeholder scenario, the active identity remains `E07-vm-job-calls-exec-service`, and D7/wire assertions remain with S-GTI-02/S-GTI-03 and Rust tests. |
| M-02 — successful narrated evidence | **STAYS RESOLVED** | E07's stub still exits 75. `run-expectation.sh:85-140` records it as pending and now returns success only for `execution_status: succeeded`; absent and failed runners also fail closed. |
| L-01 — catalogue navigation says all runners use Lima | **RESOLVED** | `verification/README.md:110-128` now says the declared substrate and documents fail-closed pending/absent results. `verification/expectations/INDEX.md:183-186` distinguishes Lima's helper from native-metal declarations. |

### Current product-boundary assessment

| Required boundary | Result | Evidence |
|---|---|---|
| Exactly one feature expectation, E07 | PASS | One active E07 directory/index row/roadmap identity; no active former-E07, E08, or E09 expectation identity |
| Exactly one checked-in operator journey | PASS | One feature bundle with two specs, two helper sources, one preparation entry point, and one operator entry point |
| One VM Job calls one Exec Service | PASS | `caller.toml` is the sole `[job]+[vm]`; `callee.toml` is the sole `[service]+[exec]`; request and reply constants match exactly |
| Callee availability is observed on a real public surface | PASS | Dedicated Service parser accepts only `Alloc/State=Running` and requires replicas `1/1`; parser self-check rejects Job/Service cross-acceptance |
| Caller success is causally reply-dependent | PASS | Dedicated Job parser requires `Attempt/State=Terminated`, public `Verdict: Succeeded`, and the helper exits zero only for the exact response |
| Example is operator-runnable without phantom fixtures | PASS | Shared preparation materializes every unavoidable path from checked-in sources and qualified kernel/rootfs inputs |
| KEK handling does not damage ambient state | PASS | Fresh anonymous session verifies the production description is absent; no ambient purge/overwrite remains, and the session dies with `serve` |
| Cleanup stays outside product-private assertions | PASS | Public stop results, exact started process identity, and invocation-token-owned materialization only; no private residue probes or repairs |
| E08/E09 and internal guarantees remain Rust-only | PASS | Active DISTILL, feature delta, catalogue, and pending roadmap assign wire/kernel/failure/recovery/cleanup/replay guarantees to Rust |
| Pending evidence cannot masquerade as success | PASS | Exit 75, absent runner, invalid substrate, and arbitrary failure all return nonzero; only execution status `succeeded` returns zero |
| E06 compatibility is preserved | PASS | Native-metal metadata is checked in, the runner's driving logic is unchanged, historical manifest semantics are documented, and fresh native-metal manifest behavior is exercised host-safely |

### Harness and E06 compatibility evidence

- `verification/harness/test-run-expectation.sh` passed all six host-safe
  branches: successful default/Lima, successful native-metal, exit-75 pending,
  arbitrary failure, absent runner, and invalid substrate.
- E06's new `execution-substrate` contains exactly `native-metal`. Its runner
  changes are explanatory only; its metal transport, default-feature build,
  evidence capture, and existing satisfied evidence files are not rewritten.
- E06's README and catalogue entry distinguish the historical pinned
  `executed_in_lima: true` field from accurate fresh
  `execution_substrate: native-metal` / `executed_in_lima: false` manifests.
- The global catalogue instructions now route authors by declared substrate,
  closing Iteration 2's only low finding without making Lima a fallback for
  E06 or E07.

### Verification performed

- Read the complete prior artifact, both remediation commits, every file
  changed by `5279a561fcff74f61e8329f07fb6a72af0abe051`, and the current active
  E07/example/catalogue/roadmap state.
- Confirmed by immutable-tree listing and exact searches that this feature has
  only E07; the old E07 slug and E08/E09 expectation identities are absent from
  active artifacts.
- Ran `bash -n` across the feature scripts, E06/E07 runners, harness, and new
  harness branch test; all passed.
- Ran `shellcheck` across the same changed scripts; all findings passed, with
  E06's established dynamic `lima-helpers.sh` source checked with SC1091
  excluded.
- Ran `prepare.sh check-source` and `run-example.sh check-source`; both passed,
  including the Service/Job parser separation checks.
- Ran the host-safe harness branch suite; all six cases passed.
- Executed the E07 stub directly and observed pending-only output plus exit 75.
- Compiled both checked-in helpers as Rust 2024 metadata; both passed.
- Parsed the roadmap with `jq`, confirmed `validation.status = pending` and
  `requires_regeneration = true`, and confirmed its only E07/E08/E09-prefixed
  identity is `E07-vm-job-calls-exec-service`.
- Confirmed `git show --check` is clean for the target commit.
- Native-metal execution and mutation testing were not run, as required. No
  E07 evidence was created or claimed.

### Final verdict

**APPROVED** — zero open blocker, high, medium, or low findings. The sole-E07
product boundary, operator-runnable example, fail-closed harness behavior, and
E06 native-metal compatibility are coherent at the cumulative target.

---

## Iteration 4 — Launch-lifecycle and product-boundary re-review

### Metadata

| Field | Value |
|---|---|
| Target commit | `f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Cumulative review range | `9558af759e049564b7149bea6459961979899e96..f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Remediation commit inspected | `f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Review type | Same-reviewer PRODUCT remediation re-review |
| Review date | 2026-08-29 |
| Verdict | **APPROVED** |

### Verdict summary

All prior findings remain closed. The launch-lifecycle remediation changes
only the operator example, its host-safe lifecycle test, and the documents
that bind them. It does not add a product implementation, a product-private
observer, or another expectation. The active feature still has exactly one
expectation, E07, for one VM Job calling one Exec Service and succeeding only
after the exact reply. D7, failure, recovery, reclamation, stop/idempotency,
kernel cleanup, and replay remain Rust-owned; E08 and E09 do not reappear as
expectations.

The new `keyctl` use is correctly confined to operator/example setup. The
wrapper creates a fresh anonymous session, checks that it is accessible, and
checks the fixture precondition that the production KEK description is absent.
It neither installs nor reads the KEK, derives key material, implements the
production fallback order, or treats the precondition check as product
evidence. The checked-in 32-byte credential is delivered through
`CREDENTIALS_DIRECTORY`; the built `serve` path still composes
`SystemdCredsKeyring`, whose production Rust performs search, delivery,
fold/add, and read-back. No `crates/` path changed anywhere in the cumulative
review range.

### Finding counts

| Severity | Open | Previously resolved and still closed |
|---|---:|---:|
| Blocker | 0 | 1 |
| High | 0 | 1 |
| Medium | 0 | 2 |
| Low | 0 | 1 |

### Prior-finding dispositions

| Finding | Iteration 4 disposition | Evidence |
|---|---|---|
| B-01 — inferred startup probe fails the callee | **STAYS RESOLVED** | `callee.toml:17-22` retains `startup = []`; `run-example.sh:282-299` accepts only the public Service allocation `Running` with replicas `1/1` before caller deployment. |
| H-01 — phantom preparation | **STAYS RESOLVED** | `prepare.sh:200-263` still materializes both checked-in helpers and every qualified kernel/rootfs/credential path. `run-example.sh:320-365` remains the bounded operator entry point using that shared preparation. |
| M-01 — public anchor retains private wire claims | **STAYS RESOLVED** | S-GTI-01 remains reply-only at `test-scenarios.md:230-234`; E07 excludes D7, nft/netlink, capture/counters, TLS/kTLS, generation, wire, and private cleanup at `README.md:73-77`. |
| M-02 — successful narrated evidence | **STAYS RESOLVED** | The E07 runner still performs only `check-source`, prints pending text, and exits 75. Direct execution returned 75, and the harness branch suite retained fail-closed pending behavior. |
| L-01 — catalogue navigation says all runners use Lima | **STAYS RESOLVED** | The catalogue still documents declared-substrate execution and separately lists the host-safe E07 lifecycle test; E07 continues to declare `native-metal`. |

### Keyctl and product-boundary assessment

| Question | Result | Evidence |
|---|---|---|
| Is `keyctl` confined to operator/example setup? | PASS | The only executable uses in the feature bundle are `keyctl session -`, `keyctl describe @s`, and `keyctl search @s user <description>` in `session-wrapper.sh:63-89`. The E07 runner remains pending and invokes none of them. |
| Does the shell duplicate production KEK behavior? | PASS — no duplication | The shell never adds, reads, hashes/folds, purges, revokes, or resolves a KEK. `run-example.sh` passes only `CREDENTIALS_DIRECTORY`; `overdrive-cli` composes `SystemdCredsKeyring::new()` at `serve.rs:112-122`, and its Rust `resolve` owns search → delivery → add → read-back at `keyring.rs:363-393`. |
| Does keyring setup become E07 evidence? | PASS — no | The fresh-session absence check is a fixture-isolation precondition. E07 success remains solely the public Service `Running 1/1` plus reply-dependent Job `Terminated` / `Succeeded` result. |
| Does lifecycle cleanup inspect product-private state? | PASS — no | `session-lifecycle.sh` observes only the harness-owned direct wrapper, private process group, Linux start time, and token files. It does not inspect allocation processes, run directories, cgroups, namespaces, links, nft/FIB, or capture state. |
| Does the host-safe test introduce a second product contract? | PASS — no | `test-e07-session-lifecycle.sh` invokes neither `keyctl` nor the product. It fault-tests only wrapper ownership, bounded signalling/reaping, and unrelated-process preservation. |

### Current product-boundary assessment

| Required boundary | Result | Evidence |
|---|---|---|
| Exactly one feature expectation, E07 | PASS | The immutable target tree contains only `verification/expectations/E07-vm-job-calls-exec-service/`; active mappings contain no former-E07, E08, or E09 expectation identity. |
| Exactly one VM Job and one Exec Service | PASS | The example TOML inventory has one each of `[job]`, `[vm]`, `[service]`, and `[exec]`; request/reply constants remain byte-identical across the two checked-in helpers. |
| Public product outcome remains narrow | PASS | Built `serve`, `deploy`, `workload describe`, and `job stop` remain the product-driving surface; success is causally dependent on the exact reply. |
| Harness lifecycle stays harness-owned | PASS | The new wrapper/lifecycle scripts bound the process that the example itself starts; they do not assert or repair Overdrive's internal allocation cleanup. |
| E08/E09 and internal contracts remain Rust-only | PASS | Active DISTILL and feature-delta mappings keep D7/wire, failure, recovery, reclamation, stop/idempotency, cleanup complements, and replay in Rust with no E08/E09 catalogue entry. |
| Evidence remains honest | PASS | E07 is still `pending`, its stub exits 75, the roadmap remains pending/regeneration-required, and no native-metal result or evidence is claimed. |

### Verification performed

- Read the complete prior artifact, the full `f908e0cf` commit, every changed
  lifecycle/example/document path, the active E07 contract, and the production
  `SystemdCredsKeyring` composition and resolve boundaries.
- Confirmed the cumulative range changes no `crates/` file and that the only
  added executable `keyctl` operations are fresh-session creation plus
  accessibility/absence checks.
- Confirmed by immutable-tree inventory and active-artifact searches that this
  feature contains exactly one E07 expectation and no E08/E09 expectation.
- Ran `bash -n` and `shellcheck` across the example scripts, E07 runner,
  lifecycle test, and generic harness scripts; all passed.
- Ran `prepare.sh check-source` and `run-example.sh check-source`; both passed.
- Ran `verification/harness/test-e07-session-lifecycle.sh`; all seven
  host-safe fault cases passed, including bounded TERM/KILL and unrelated
  sentinel preservation.
- Ran `verification/harness/test-run-expectation.sh`; all status/substrate
  branches passed.
- Executed the E07 stub directly and observed pending-only output with exit 75.
- Compiled both checked-in helpers as Rust 2024 metadata; both passed.
- Parsed the roadmap with `jq`: `validation.status = pending`,
  `requires_regeneration = true`, and the only E07/E08/E09-prefixed inventory
  identity is `E07-vm-job-calls-exec-service`.
- Confirmed `git show --check` is clean for the target commit.
- Native-metal execution and mutation testing were not run, as required. No
  E07 evidence was created or claimed.

### Final verdict

**APPROVED** — zero open blocker, high, medium, or low findings. Commit
`f908e0cf` closes the launch-lifecycle gap without broadening E07, leaking
`keyctl` into product behavior, duplicating the production KEK implementation,
or reintroducing E08/E09 as expectations.
