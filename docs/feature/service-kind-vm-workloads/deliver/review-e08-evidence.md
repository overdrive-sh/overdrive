# E08 Evidence Audit — Different-Fox Review

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Roadmap step | `02-02` — E08 walking skeleton |
| Expectation | `E08-vm-service-guest-health` |
| Audit role | Independent evidence auditor (different fox) |
| Audit date | 2026-09-09 |
| Evidence substrate | `native-metal` |
| Qualifying capture | 2026-09-06T20:58:28Z–20:59:14Z |
| Pinned baseline SHA | `37f1f6d84a5912ccd1048f8604ba90026b7e7d1e` |
| Runner result | exit `0`, `succeeded` |
| Verdict | **SATISFIED** |

The implementation review for step 02-02 is recorded as `APPROVED` after its
second iteration in `deliver/review-02-02.md`. That is implementation-review
status only; it is not used as evidence for any claim below.

## Evidence-only boundary

This audit evaluates the checked-in E08 black-box evidence and its provenance.
It does not review or re-derive the implementation, tests, examples, or
patches that produced the evidence. The retained dirty patches were hashed but
their contents were not read. No Cargo, test, expectation, or mutation command
was run, and no crate was imported or linked.

The only file written by this audit is this Markdown artifact. The verification
status in the expectation README and index remains unchanged; the exact status
action is recorded at the end for the orchestrator.

## Contract and anchors audited

The approved roadmap contract for step 02-02 requires the E08 walking skeleton
to activate a healthy VM Service through the built product, order `Accepted`
before `Stable`, show guest TCP/HTTP results, return the exact peer-VM Job
reply, and report zero teardown deltas. Its evidence contract requires the
black-box harness and an independent audit.

The E08 README identifies the public journey and anchors:

- `S-SVM-01`
- `US-SVM-1`, `US-SVM-2`
- `ADR-0090`, `ADR-0091`
- `K1`, `K2`

The README's boundary is the built default-feature product: a canonical
native-metal lease, `target/debug/overdrive serve`, one service deploy, service
describe, a peer Job deploy, client describe, and cleanup. Its oracles require
the service's accepted-to-stable ordering, TCP/HTTP results, exact
`SVM-E08-GUEST-OK`, and zero owned VM/probe/network/cgroup/run-directory/
mount/loop/preparation deltas.

## Evidence files examined

### Final qualifying capture

All files in the final E08 evidence directory were examined as evidence,
except that the dirty patch was only hashed:

- `verification/expectations/E08-vm-service-guest-health/evidence/verification.yaml`
- `.../evidence/product-run.meta`
- `.../evidence/product-run.out`
- `.../evidence/run.log`
- `.../evidence/dirty-status.txt`
- `.../evidence/execution-substrate.txt`
- `.../evidence/dirty-diff.patch` (hash and size only)

The final `verification.yaml` records expectation `E08`, date
`2026-09-06T20:58:28Z`, baseline and harness SHA
`37f1f6d84a5912ccd1048f8604ba90026b7e7d1e`, dirty worktree, seed `1`, the
`verification/harness/run-expectation.sh E08` invocation, `runner_invoked: true`,
`execution_status: succeeded`, `execution_substrate: native-metal`,
`executed_in_lima: false`, and runner exit `0` (`verification.yaml:1-16`).

The final `product-run.meta` records the harness command, start and finish
times, and exit `0` (`product-run.meta:1-5`). The final `product-run.out` and
`run.log` are byte-identical (comparison exit `0`); both contain the complete
captured stream described below. The final execution-substrate declaration is
`native-metal`.

### Retained earlier attempt

The pre-remediation attempt is retained in Git history rather than in an
attempt-named directory. I examined its evidence files from commit
`37f1f6d84a5912ccd1048f8604ba90026b7e7d1e`:

- historical `verification.yaml`
- historical `product-run.meta`
- historical `product-run.out`
- historical `run.log`
- historical `dirty-status.txt`
- historical `dirty-diff.patch` (hash and size only)

The historical receipt records a successful native-metal runner exit, but its
service `Accepted.` was outside the service deploy PTY transcript. It is
therefore retained for failure transparency and explicitly excluded from the
qualifying verdict.

The E08 README, E08 runner, verification README and index/status conventions,
the approved roadmap entry, and `deliver/review-02-02.md` were also read. The
last of these establishes implementation-review status only, as stated above.

## Provenance and capture integrity

The final receipt is internally pinned:

| Artifact | Recorded value |
|---|---|
| `verification.yaml` SHA-256 | `985f37eead098cefa854b4817dcf273c273a58742cdf84b71df37dcdc3939806` |
| `product-run.meta` SHA-256 | `b68bb07dc0af5c5e6117252a175d3dfff6234164e52d111082ae1541c4ccc79c` |
| `product-run.out` / `run.log` SHA-256 | `505cbd6944dc027c8c7ddcdf3f3f82943ad95e187601f03fcce0b2a0fbe40c25` |
| `dirty-status.txt` SHA-256 | `a255e1452efa20bfca0aff5ae662b8592db2bba8fe6c9299e6564657b1e169e1` |
| `dirty-diff.patch` SHA-256 | `b4d4a35d82809b7df446cf00bbffba078031a1525a2360c6bd6f0f019ed89ab0` |
| final dirty patch size | `171387` bytes |

The final dirty-status receipt lists the dirty production/example/evidence
paths and retained review/design artifacts (`dirty-status.txt:1-21`). The
dirty flag and patch hash make the captured source state reproducible without
concealing that the run was not from a clean worktree. The audit did not read
the patch contents.

The E08 runner invokes the checked-in product example through the native-metal
harness and requires the product's exact E08 pass sentence; it does not invoke
Cargo tests, a Rust test binary, or an overdrive crate. The final product stream
shows the product build and then a direct `target/debug/overdrive deploy`
command (`product-run.out:1-23`). This is evidence of the required external
product boundary and contains no test-harness execution path.

## Claim-by-claim verdicts

### 1. One fresh, un-detached service deploy orders `Accepted` before `Stable`

**SATISFIED.** In the final capture, the service deploy PTY starts at
`product-run.out:23` with the service `target/debug/overdrive deploy .../service.toml`
command. The same PTY records exactly one service `Accepted.` at line 24,
the service identity and outcome at lines 25-30, and exactly one
`Service 'service-vm-e08' is stable` at line 31. The PTY closes successfully at
line 33. Thus the service's single transcript has the required
`Accepted → Stable` ordering; the later `Accepted.` at line 55 belongs to the
separate peer-client Job deployment, not the service PTY.

The final stream has no failure block between the service PTY start and its
successful close (`product-run.out:23-33`). The capture is a distinct later
run than the retained failed attempt and is the qualifying fresh capture.

### 2. The runner drove the built default-feature product at the black-box boundary

**SATISFIED.** The final evidence declares `runner_invoked: true`, native metal,
not Lima, and runner exit `0` (`verification.yaml:1-16`). The product stream
shows the build and direct `target/debug/overdrive` command
(`product-run.out:1-23`). The E08 runner's recorded invocation is the checked-in
example through `cargo xtask metal run`, and its oracle requires the exact E08
pass sentence; no Cargo test, test binary, crate import, or in-process
expectation runner appears in the captured product path. This satisfies the
required external built-product boundary without demanding an unapproved extra
feature-print assertion.

### 3. Guest TCP/HTTP results and Service identity/lifecycle are present

**SATISFIED.** The service describe output identifies `kind Service`, digest,
and `replicas 1/1` (`product-run.out:34-38`). It identifies the allocation as
`alloc-service-vm-e08-0`, state `Running`, restarts `0`, and reason `driver
started` (`product-run.out:37-40`). It records the effective guest address
`10.99.128.2`, VIP, and TCP listeners on ports `18080` and `18081`
(`product-run.out:41-46`). It also records the issued SPIFFE workload identity,
issuer serial, and expiry (`product-run.out:47-51`).

The guest TCP startup probe is `last=pass`, and the guest HTTP readiness probe
is `last=pass` (`product-run.out:52-54`). The wildcard bind shown in those probe
targets is permitted by the E08 contract; the effective guest address is
separately present at line 42.

### 4. The peer VM Job receives the exact byte reply through the Service frontend

**SATISFIED.** The second deploy is identified as
`service-vm-e08-client`; its Job describe output reports `kind Job` and
`Verdict Succeeded` (`product-run.out:55-67`). The required oracle sentence is
present verbatim: `E08 PASS: peer VM Job received byte-exact
SVM-E08-GUEST-OK through Service frontend` (`product-run.out:69`). This is the
checked-in runner's external oracle, not a Rust test assertion or an inline
reimplementation of the workload.

### 5. Teardown leaves every required owned delta at zero

**SATISFIED.** The final capture records client and service stop receipts and
removal of the E08-owned materialization (`product-run.out:70-74`). It then
reports all required categories as zero in one teardown receipt:

`vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0`

(`product-run.out:75`). No category named by the E08 contract is omitted.

### 6. Receipts, anchors, dirty provenance, and retained failures are complete

**SATISFIED.** The final verification receipt contains all required E08
anchors, runner invocation, seed, baseline/harness SHA, substrate, dirty
state, and exit status (`verification.yaml:1-16`). The command/timestamp/exit
receipt is complete in `product-run.meta:1-5`; the merged product transcript is
preserved in both `product-run.out` and `run.log`; and the dirty status and
patch digest are preserved as documented above.

The earlier attempt is not silently discarded. Its historical transcript shows
`Accepted.` before the PTY begins, then only `Stable` inside the service PTY;
its historical `verification.yaml` and metadata report the earlier time,
baseline/harness provenance, native-metal substrate, and exit `0`. It is
therefore visible, reproducibly identified, and rejected for this claim rather
than being counted as a passing capture.

### 7. Startup-failure ownership is not an additional E08 black-box oracle

**NON-FINDING / SCOPE BOUNDARY.** The roadmap step also names an implementation
criterion about preserving `Running` across startup failure. The E08 expectation
contract is the healthy walking-skeleton black-box journey; it does not require
the expectation runner to manufacture or assert that internal failure path.
This evidence audit does not use implementation tests or source review to
claim that behavior, and does not demand an unapproved black-box assertion.
The separate implementation review status is recorded above but is not proof
for this evidence audit.

## Retained-attempt disposition

| Attempt | Evidence and disposition |
|---|---|
| Historical pre-remediation run, 2026-09-06T20:16:47Z–20:17:21Z, commit `37f1f6d...` | **EXCLUDED.** The service `Accepted.` is outside the service PTY; the PTY begins later and contains `Stable` without its preceding `Accepted`. Runner exit `0` is not treated as a satisfied verdict. Historical output/run-log SHA-256: `05e3fd4c6d55d7c51de8bcb4d7543e0a6e5feab136149ac4171af779f8fee800`. Historical dirty patch hash (contents not read): `0137ac3617de8298ec3c70d95d2e8021edeb5b4590a25c17014777bba23df38f` (80,144 bytes). |
| Final run, 2026-09-06T20:58:28Z–20:59:14Z, baseline `37f1f6d84a5912ccd1048f8604ba90026b7e7d1e`, dirty patch recorded | **QUALIFYING.** One service PTY contains `Accepted` then `Stable`; guest probes, identity/lifecycle, exact peer Job reply, zero teardown, and provenance receipts all pass the claim-by-claim audit above. |

The old attempt is retained in Git history and explicitly dispositioned; it is
not overwritten, hidden, or silently promoted by the later capture.

## Non-findings

- The separate client `Accepted.` at `product-run.out:55` does not violate the
  service sequence because it is outside the service PTY bounded by lines
  23-33 and belongs to the peer Job journey.
- Wildcard probe targets `0.0.0.0` are not a defect: wildcard/omitted probe
  targets are part of the E08 contract, and the effective guest address is
  recorded independently at line 42.
- `product-run.out` and `run.log` being identical is expected preservation of
  the harness stream, not duplicate execution; the comparison was read-only.
- `executed_in_lima: false` is consistent with the required native-metal
  substrate.
- No mutation testing was run, as explicitly directed. Mutation testing is not
  required to decide this evidence-only verdict.

## Read-only audit commands

The audit used only read-only inspection and hashing, including file listing and
line-numbered reads of the contract, runner, status conventions, final
evidence, and retained historical evidence; Git history/object reads for the
retained evidence files; `git status`, `git rev-parse`, and `git log`; a
byte-comparison of `product-run.out` and `run.log`; and SHA-256/size checks for
the evidence receipts and dirty patches. The retained dirty patch contents were
not read. No `cargo`, `nextest`, expectation runner, production binary,
mutation test, or crate import/link operation was executed.

## Final verdict and exact documentation action

**SATISFIED.** The final native-metal evidence is a complete, pinned,
black-box capture with one service deploy PTY showing `Accepted → Stable`, the
guest TCP/HTTP and identity/lifecycle surfaces, the exact peer-VM Job reply,
zero teardown deltas, and complete dirty/SHA receipts. The failed historical
capture is retained and explicitly excluded, so it cannot conceal the original
contract violation.

After this audit is accepted, the orchestrator should make exactly this
documentation status change (and no evidence or implementation change):

1. In `verification/expectations/E08-vm-service-guest-health/README.md`, change
   `Status: pending (DISTILL RED handoff)` to `Status: satisfied` and link this
   audit artifact at
   `docs/feature/service-kind-vm-workloads/deliver/review-e08-evidence.md`.
2. In `verification/expectations/INDEX.md`, change the E08 row and its nearby
   feature-coverage entry from `pending` to `satisfied`, linking this audit.

The auditor does not apply that status update.
