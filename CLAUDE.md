## Development Paradigm

This project follows the **object-oriented** paradigm. Use @nw-software-crafter for implementation.

## Implement to the design — never invent API surface

When implementing against an accepted design (an ADR, `brief.md`, a
feature-delta, a roadmap step), match the design's **exact public API
shape**. Do **not** invent new public surface — a new method, type, enum
variant, trait, or parameter — to make tests green or to fill a gap the
design left underspecified. The design is a contract, not a suggestion;
an implementation that adds API the ADR did not call for has *diverged*,
even if every test passes.

When the design specifies a *model* but not the exact *signature* (e.g.
"the transient is the step's `Err` re-driven by the engine" without the
function shape), the gap is **not** licence to improvise. **STOP and
surface the gap** to the user / orchestrator and get the shape pinned —
never reach for the nearest mechanism that compiles. A subagent that
grades itself on "tests green" will invent surface; that is the failure
mode this rule exists to prevent.

This binds three roles:

- **Crafters**: build only the API the design names. If you need a
  primitive the design doesn't specify, return a blocker — do not add a
  public method/type/variant on your own initiative.
- **Orchestrators dispatching crafters**: for any design-sensitive
  surface, pin the **exact signature** in the dispatch and explicitly
  forbid inventing API. Granting latitude ("pick the cleanest shape,"
  "add a variant if needed") *causes* divergence — do not.
- **Reviewers / orchestrators accepting work**: verify the output
  against the design's API shape, not just "tests pass." A green suite
  over a divergent API is a rejection, not an approval.

**Precedent** (the `workflow-result-error-model` feature, ADR-0065):
crafters twice invented surface the ADR did not sanction — a
`TerminalErrorKind::Retryable` variant (a "terminal error" that wasn't
terminal, flatly contradicting the ADR's "retryable never reaches the
return type"), then a second `ctx.run_retryable` step method instead of
the ADR's single `ctx.run`. Both compiled and passed their tests; both
were design divergences caught only in adversarial review and by the
user, and both cost a rework cycle. The cost of surfacing a gap is one
message; the cost of inventing past it is a wrong contract that
propagates until someone notices.

## Repository structure

Workspace crates live under `crates/` (plus `xtask/` for build tooling).
Each crate declares `package.metadata.overdrive.crate_class` per
ADR-0003; the dst-lint gate (`xtask/src/dst_lint.rs`) scans only
`crate_class = "core"` crates for banned real-infra calls.

| Crate | Class | What's in it |
|---|---|---|
| `overdrive-core` | `core` | Newtypes, error types, **port traits** (`Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `IntentStore`, `ObservationStore`, `Llm`). No I/O. No `tokio`, `rand`, or `std::net` in the dependency graph — if it would fail dst-lint, it can't live here. |
| `overdrive-host` | `adapter-host` | Production bindings from the core port traits to the host OS / kernel / network (`SystemClock`, `OsEntropy`, `TcpTransport`, etc.). Reconciler and policy crates MUST NOT depend on this — depending on `overdrive-host` is the explicit opt-in to real I/O. |
| `overdrive-sim` | `adapter-sim` | `Sim*` bindings for the same traits, the turmoil DST harness, and the invariant catalogue. Owns `turmoil` and `StdRng` — nothing else should. See ADR-0004. |
| `overdrive-store-local` | `adapter-host` | `LocalStore` (single-node `IntentStore` over `redb`). |
| `overdrive-testing` | `adapter-host` | Shared real-infra test fixtures (netns, veth, topology). Linux-only items. Dev-dependency only — never `[dependencies]`. See `.claude/rules/development.md` § "Shared real-infra test fixtures — overdrive-testing". |
| `overdrive-cli` | `binary` | Operator CLI entry point; `eyre`-based error reporting. |
| `xtask` | `binary` | Build/lint/DST runner (`cargo xtask …`). Allowed to touch the filesystem and wall-clock; not scanned by dst-lint. |

**The sim/host split is load-bearing, not cosmetic.** `overdrive-core`
depends only on the trait surface; wiring crates (future
`overdrive-node`, `overdrive-control-plane`, gateway) pick host impls
for production and sim impls under tests. Anything that would put
`tokio::net::*`, `Instant::now`, or `rand::thread_rng` on a
`core`-class compile path fails dst-lint at PR time.

The four valid crate classes are `core | adapter-host | adapter-sim |
binary` — nothing else. Adding a new crate means picking one of those
up front and declaring it in the crate's `Cargo.toml`.

Non-code layout:

- `docs/whitepaper.md` — SSOT for platform design (§ references in
  ADRs and rules point here).
- `docs/product/architecture/adr-*.md` — accepted architectural
  decisions. Editing an ADR or supersession goes through the
  architect agent, not inline.
- `docs/product/architecture/brief.md`, `docs/product/commercial.md`
  — SSOT for scope and commercial shape (tenancy, licensing, tiers).
- `docs/feature/{slug}/{wave}/…` — in-flight nWave artifacts (discuss
  → distill → design → deliver). Temporary; archived into
  `docs/evolution/` when the feature is finalised.
- `docs/research/` — research notes and evidence.
- `.claude/rules/{development,testing}.md` — project-wide Rust and
  testing discipline. These override defaults for agents working in
  this repo.

## Rust library conventions

Every library crate that defines its own error type also exposes a matching
`Result` alias alongside it, so call sites never have to name the error type:

```rust
/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
```

Usage:

- Internal code writes `fn foo(...) -> Result<Foo>` (no error generic), and
  `?` propagates anything that converts via `thiserror`'s `#[from]`.
- Cross-crate callers either write `overdrive_core::Result<T>` or import
  the alias. They never re-declare `std::result::Result<T, SomeError>`.
- Override the default when a function returns a different error type
  explicitly: `fn bar() -> Result<Bar, OtherError>`.
- Binary boundaries (`overdrive-cli`, `xtask`) drop this pattern and return
  `eyre::Result<T>` instead — see `crates/overdrive-cli/src/main.rs`.

This keeps the typed-error discipline from `.claude/rules/development.md`
intact while removing the noise of repeating the error type at every call
site.

## CLI verb — `overdrive deploy <SPEC>`, never `overdrive job submit`

The operator command to apply a workload TOML spec is **`overdrive
deploy <SPEC>`** (e.g. `overdrive deploy dns-resolver.toml`). It is a
top-level `Command::Deploy { spec, detach }` in
`crates/overdrive-cli/src/cli.rs`.

`overdrive job submit` **does not exist** as a user-facing verb. It was
the real command until commit `17f633e2` ("refactor: rename
job-specific identifiers to workload-generic naming", May 11 2026),
which promoted it to top-level `overdrive deploy`. The `Job` subcommand
now carries only `list` and `stop`. The internal handler surface was
renamed to track the verb in #193: the module is
`crates/overdrive-cli/src/commands/deploy.rs` (`commands::deploy`), the
handlers are `deploy` / `deploy_streaming` (plus the per-workload-kind
`deploy_streaming_job` / `deploy_streaming_service`), the arg/output
types are `DeployArgs` / `DeployOutput` / `DeployStreamingOutput`, and
the integration tests live in `tests/integration/deploy.rs`. No
`job::submit` / `SubmitArgs` surface remains in the crate. When writing
operator-facing docs, journeys, or examples, the only correct verb is
`overdrive deploy <SPEC>`. Do not copy `job submit` from older phase
docs.

## Two distinct certificate authorities — do not conflate them

`overdrive serve` wires **two independent CAs**. They share nothing but
the `rcgen` library and are easy to mix up (this has already cost a
debugging detour):

- **Operator / control-plane HTTPS CA** —
  `tls_bootstrap::mint_ephemeral_ca()`
  (`crates/overdrive-control-plane/src/lib.rs:1392`). Backs the operator
  mTLS surface the CLI connects to (Talos-shape operator auth, D-CA-5 /
  [#81](https://github.com/overdrive-sh/overdrive/issues/81)). **Ephemeral
  by design and staying that way** — it is NOT the workload-identity root
  and is out of scope for any built-in-CA / SVID work.
- **Workload-identity CA** — `RcgenCa::new(OsEntropy, ca_subject)`
  (`lib.rs:1754`), now backed by the **persistent, KEK-sealed root**
  `ca_boot::boot_ca` (`lib.rs:1768`, ADR-0063 /
  [#215](https://github.com/overdrive-sh/overdrive/issues/215) boot-side,
  closes D-OC-4). Signs the SVIDs issued to workloads. On boot, `boot_ca`
  runs the Earned-Trust probes (KEK-resolve, envelope-decrypt) then
  generate-or-adopt: the first boot generates the P-256 root, envelope-seals
  the key under the operator KEK (injected via `config.kek` —
  `SystemdCredsKeyring` in production, `SimKek` under test) and persists it
  to the `IntentStore`; every later boot decrypts and adopts the SAME root.
  A boot failure propagates as the typed `ControlPlaneError::CaBoot` and
  refuses to start (never flattened to `Internal`). This was a **single-cut
  replacement of the prior ephemeral per-boot root** — there is no longer an
  ephemeral workload root; the `RcgenCa` holds the adopted persistent
  root/intermediate and the `IssueSvid` executor mints leaves off it.

When reasoning about "the persistent CA," "boot the CA," "root key at
rest," or any SVID-issuance path, the subject is the **workload-identity**
CA (`boot_ca` / `RcgenCa`), never `mint_ephemeral_ca`. Only the
operator/control-plane HTTPS CA is ephemeral now; the workload root is
persistent and KEK-sealed. "`serve` boots the ephemeral CA" is true ONLY
of the operator CA — name which one, or the distinction is lost.

## "Cert rotation workflow" = external ACME, NOT internal SVID reissue

The **cert-rotation `Workflow`** named as the canonical/first workflow
example — whitepaper §18, `.claude/rules/workflows.md` § "Codebase
precedent", and the `cert_rotation` body in `.claude/rules/development.md`
§ "Workflow contract" — refers to **external ACME / public-certificate**
rotation: the genuine four-step `request → wait-for-DNS-propagation →
validate → publish` sequence. Multiple ordered side-effecting steps plus a
real propagation wait → genuinely workflow-shaped (the textbook Bar-2
case).

It does **NOT** describe **internal workload-SVID reissue**. Minting a
workload SVID is a *single* synchronous `Ca::issue_svid` call. By
workflows.md's own decision rule a single idempotent side-effecting call
is a **reconciler action** (`Action::IssueSvid`, already live in
`SvidLifecycle`), not a workflow — "only when the reconciler would
coordinate three or more external calls that must complete as a unit does
the sequence cross into workflow territory." Internal SVID reissue has no
DNS wait, no validate/publish, and (pre-Phase-5 revocation) no second
step to coordinate.

GH [#40](https://github.com/overdrive-sh/overdrive/issues/40) ("cert
rotation as first internal workflow") and ADR-0067's #40-boundary section
**conflated the two** — they pasted the external "wait-for-DNS-propagation"
shape onto internal SVID rotation, which has no such step. If you see
internal SVID rotation described as a journaled `ctx.run(...)` workflow,
that is the conflation; the distinction above governs. (#40 / ADR-0067 /
the workflows.md precedent still carry the conflated wording pending
correction.)

## Workload identity model — workloads hold NOTHING; the kernel does mTLS

mTLS in Overdrive is **kernel-mediated** (eBPF sockops + kTLS,
[#26](https://github.com/overdrive-sh/overdrive/issues/26)). The
consequences, easy to get wrong:

- **Workloads are identity-unaware and hold NO SVID material** — no cert,
  no key. They open ordinary sockets; the kernel-side BPF programs
  terminate/originate TLS transparently using material the platform
  supplies. There is no SPIRE-agent-style workload-held copy.
- **The worker / control-plane `IdentityMgr` holds the `SvidMaterial`**
  (cert PEM/DER + `leaf_key` + serial + `not_after`) **in memory**
  (`Arc<RwLock<BTreeMap<AllocationId, SvidMaterial>>>`); the kernel is the
  consumer.
- **The durable `issued_certificates` audit row persists only the
  *facts*** — `spiffe_id, serial, issuer_serial, not_before, not_after,
  node_id, issued_at` (`crates/overdrive-core/src/ca/issued_certificate_row.rs`).
  It carries **no cert bytes and no private key**, so it cannot reconstruct
  a usable SVID.

Implications when reasoning about restart/rotation: **do not say "the
workload still holds a valid SVID"** — it never held one. On a
control-plane restart the in-memory hold (`IdentityMgr`) is lost; only the
audit-row facts survive. Whether the *operative* crypto survives a CP
restart (kernel-held / pinned, à la the bpffs HoM pinning discipline) or
must be re-supplied by the control plane is a **#26-coupled, Tier-3-spike
question** — do not assume it without confirming the kernel/kTLS survival
semantics on a real kernel.

## Mutation Testing Strategy

This project uses **per-feature** mutation testing. Per-PR runs are diff-scoped via `cargo mutants --in-diff origin/main` with a kill-rate gate of ≥80%. A nightly job runs the full workspace against the baseline in `mutants-baseline/main/` to catch drift. Mutations to `unsafe` blocks, `aya-rs` eBPF programs, generated code, and async scheduling logic are excluded per `.claude/rules/testing.md`.

## nWave design dispatches — priority scope

When dispatching `@nw-solution-architect` (or any DESIGN-wave agent) for a feature whose DISCUSS/DISTILL outputs enumerate prioritised open questions (P1, P2, …), include **all priorities** by default. Do not narrow scope to P1 only unless the user explicitly says so. State "all priorities (P1 + P2 …)" in the dispatch confirmation so the scope is visible.

## Roadmap validator warnings

`des.cli.roadmap validate` flags length-limit warnings (`STEP_NAME_TOO_LONG`, `CRITERIA_TOO_LONG`, `DESCRIPTION_TOO_LONG`) that are cosmetic and non-blocking — the validator exits 0 anyway. Overdrive roadmap ACs deliberately carry scenario-level specificity (test names, invariant names, proptest targets, kill-rate thresholds), and tightening them to the defaults would lose traceability. Ignore these warnings; do not ask the crafter to trim them.

## No effort/time budget cuts

Roadmap `effort_hours` estimates are **advisory, not enforcement**. Do NOT defer scope mid-step on the basis "exceeds this slice's hour budget." Land the full work the step's acceptance criteria describe, however long it takes. If the work genuinely warrants a follow-up (e.g., a separate concern surfaces during implementation that's clearly out of the step's named scope), surface that to the user and ask — do not unilaterally ship a partial and log COMMIT EXECUTED PASS against an incomplete deliverable. The DES log is a contract; partial completions corrupt it.

## Deferrals require GitHub issues — AND user approval BEFORE creation

Every deferral surfaced during a wave dispatch — operator-tunable knobs, future-phase scope, follow-up cleanup, "we'll wire this later" — MUST be (1) surfaced to the user explicitly at the point it's introduced **and the user must approve before any issue is created or deferral language is written**, (2) if approved, captured as a GitHub issue (`gh issue create`) before the artifact lands, and (3) cited by issue number at every reference site. Hand-wavy forward pointers ("future ticket," "Phase 3+ slice," "future operator config surface") without a real issue number are forbidden — they compound across dispatches as the next reader treats the deferral as planned work and propagates the false reference.

**Agents MUST NOT unilaterally create GitHub issues.** The `gh issue create` command is a user-visible, shared-state action (§ "Executing actions with care" in the system prompt). Creating an issue without explicit user approval — even with good intent — is a violation. The correct flow is: surface the deferral to the user in a message, wait for approval, then create the issue. If the agent is a subagent that cannot message the user directly, it MUST surface the deferral as a blocker in its return message so the orchestrator can relay it.

**Clippy warnings, lint errors, and other code-quality findings are NOT deferrals — they are in-scope fixes.** Per `.claude/rules/development.md` and feedback memory: "clippy errors surfaced during a task get fixed in-scope, not classified as 'pre-existing' and left for the next person." An agent that encounters clippy `-D warnings` failures during its quality gate MUST fix them in the current commit, not create a tracking issue. The only exception is when the fix requires touching files explicitly outside the step's implementation scope AND the fix is structurally unrelated to the step's work (e.g., a lint regression in a crate the step never imported) — in that case, surface it to the user as a blocker with the specific errors and files, and let the user decide whether to expand scope or defer. Never create the issue unilaterally.

The valid moves when tempted to write "deferred to a future X":

- **Drop the deferral language.** Unstated knobs are "not in scope" by default; the reader doesn't need a promise of a future slot.
- **Fix it now.** If it's a code-quality issue (clippy, fmt, dead code, missing test) that surfaced during your step — fix it. It's not a deferral; it's your job.
- **Surface and ask.** Present the deferral to the user with the specific scope, affected files, and your recommendation. Wait for approval. On approval run `gh issue create`. Two minutes is the price of an honest forward pointer.
- **Cite an existing issue** verified by `gh issue view <N>` whose scope actually covers the deferred work — never invent, guess, or copy-paste an issue number.

Architects, crafters, and reviewers all enforce this: any of them spotting a deferral without an issue number must flag it in handoff and refuse to land the artifact until either the issue is created or the deferral language is dropped. The same applies to existing artifacts touched during a dispatch — fix the reference rather than propagate it.

This extends `.claude/rules/development.md` § "Documentation" (*No aspirational docs. Never document behaviour that is not implemented.*) to forward pointers as well as backward claims.

## Reading GitHub issues — always fetch comments (`--comments`)

When viewing a GitHub issue, **always pass `--comments`**: `gh issue
view <N> --comments` (or `gh issue view <url> --comments`). The bare
`gh issue view <N>` returns only the issue body and drops the entire
comment thread — and the comments are where the load-bearing context
usually lives: ratified decisions, scope corrections, research
findings, design-input updates, and "actually we're doing X instead"
reversals that never make it back into the original body. Reading the
body alone gives a stale or incomplete picture and leads to citing an
issue for the wrong scope.

This applies everywhere an issue is read: verifying an existing issue
before citing it (per the deferral rule above — "verified by `gh issue
view <N>`" means **`--comments`**), triaging, picking up work, or
answering a question about an issue's status. The same holds for PRs:
`gh pr view <N> --comments` when the review discussion matters. When in
doubt, fetch the comments — the cost is one flag; the cost of missing a
decision recorded only in a comment is propagating a wrong assumption.
