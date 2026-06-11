# Evolution — built-in-ca-operator-composition (GH #215 + #40 · roadmap Phase 2.6 composition)

**Finalized**: 2026-06-11 · **Feature SHA**: `e9711be5` (`chore(deliver): land
built-in-ca-operator-composition DELIVER machine artifacts`) · **ADRs**:
[ADR-0067](../product/architecture/adr-0067-workload-identity-manager-svid-lifecycle.md)
(rev 7, workload-identity-manager / SVID lifecycle) +
[ADR-0063](../product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md)
(dated amendment, built-in CA root-key protection) · **Builds on**:
[2026-06-06-built-in-ca](2026-06-06-built-in-ca.md) (the shipped X.509 hierarchy,
GH #28)

## Summary

The composition + lifecycle-completion feature that takes the already-shipped
built-in CA (ADR-0063) and SVID lifecycle manager (ADR-0067) from
**library-complete-but-unwired** to **live in the operator binary**. Three moves,
no new subsystem and no new dependency:

1. **Rotation as a reconciler ACTION (#40).** `SvidLifecycle::reconcile`'s
   near-expiry branch flips from an inert gated `StartWorkflow(cert_rotation)`
   seam to an unconditional `Action::IssueSvid` with a `"rotate-svid"`
   correlation — reusing the existing action variant unchanged. Internal SVID
   near-expiry reissue is a single mint+swap, not a ≥2-external-step workflow.
2. **Persistent CA wired into `overdrive serve` (#215 boot-side).** `run_server`
   now calls the already-implemented, already-probing `boot_ca` +
   `bootstrap_node_intermediate` (KEK-resolve probe → envelope-decrypt probe →
   adopt-or-refuse), single-cut replacing the ephemeral `RcgenCa` composition at
   `lib.rs:1595`. The Earned-Trust refuse-to-start now runs at production boot.
3. **Operator-visible current SVID (#215 consumer-side).** `overdrive alloc
   status` aggregates the append-only `issued_certificates` audit and projects
   the current cert (max-`issuance_ordinal` row per running alloc) into an
   additive `AllocStatusResponse.issued_certificates` field — `serial /
   spiffe_id / issuer_serial / not_after`, **no cert bytes, no key**.

The single load-bearing reframe this feature back-propagated: **#40 near-expiry
reissue is a reconciler ACTION, not a workflow.** Prior docs (ADR-0067, GH #40,
`.claude/rules/workflows.md` § "Codebase precedent") had pasted the external-ACME
`request → wait-for-DNS-propagation → validate → publish` four-step workflow
shape onto internal SVID rotation, which has no such step. That conflation is now
corrected at every site.

## Business context

- **Before**: the built-in CA was library-complete and proven by gated
  `integration-tests` tiers, but **deliberately not wired into the operator
  binary** (D-CA-4, prior feature). `overdrive serve` still booted the ADR-0010
  ephemeral workload-identity root (re-minted every boot, no key at rest, no
  adopt-on-restart); no operator surface exposed which SVID a running workload
  held; the near-expiry rotation branch was inert behind a `ROTATION_ENABLED`
  gate that only existed to suppress an unregistered `cert_rotation` workflow.
- **After**: `overdrive serve` boots the **persistent KEK-sealed** workload-
  identity root (sealed → persisted → adopted on restart; refuses to start
  rather than silently re-mint on a bad/absent KEK), near-expiry SVIDs rotate
  through the live `IssueSvid` action, and `overdrive alloc status` surfaces the
  current issued-cert summary per running alloc — all inside the one binary
  (no SPIRE / cert-manager / Vault).
- The two-CA discipline is intact and was re-verified in adversarial review:
  this feature touches ONLY the **workload-identity** CA (`lib.rs:1595` path).
  The operator / control-plane HTTPS CA (`mint_ephemeral_ca`, `lib.rs:1237`,
  D-CA-5 / #81) is ephemeral by design and untouched.

## Outcome KPIs (recorded here, not in `kpi-contracts.yaml`)

Per the `kpi-contracts.yaml` SSOT scope rule (that file is the `docs-platform`
feature's contract ONLY — "other features record their own outcome-KPI baselines
in their evolution records, NOT here"), this feature's KPI baselines live in this
record. These extend the inherited built-in-ca KPIs (K1–K5) to the composed
operator surface:

| KPI | Target | Measured at finalize |
|---|---|---|
| K1 (North Star — auditability) | Every issuance writes an operator-legible `issued_certificates` audit row | **Met (in-tree + O05 evidence pending #227).** The audit row is written + integration-tested (`alloc_status_issued_certificates` drives the real handler projection); the black-box operator-CLI capture (O05) is `pending` on a disposable full-system VM (#227). No clean first-GA operator-surface baseline is capturable until #227. |
| K3 (guardrail — key at rest) | 0 plaintext root-key bytes on disk across boot/restart/refuse | **Met.** D01 EDD `satisfied` (different-fox audited): the persisted envelope is KEK-sealed; the on-disk byte-scan finds no plaintext key across first-boot + adopt-on-restart. |
| O04 (guardrail — refuse-to-start) | Cause-distinct refusal (wrong-KEK / tampered / absent), no silent re-mint | **Met.** O04 EDD `satisfied` (different-fox audited): three pairwise-distinct stderr causes; the root is unchanged after a refused boot. |
| E03 (North Star — chain verifies) | 100% chain-verify under `openssl verify`, all 3 sub-claims | **Met.** E03 EDD `satisfied` (different-fox audited): chain verifies, leaf profile, AND the pathLen=0 negative anchor FAILS `openssl verify`. |

No fabricated numbers: where a clean first-GA operator-surface baseline is not
capturable (O05, blocked on #227), it is recorded as `pending` rather than
invented.

## Slices delivered (roadmap → execution-log, all 8 steps EXECUTED/PASS)

1. **Slice ① — Rotation (core action flip)** (01-01..02): `SvidLifecycle`
   near-expiry branch emits an unconditional rotate `Action::IssueSvid`
   (`"rotate-svid"` correlation, `node_id` from `running.node_id`); threshold set
   to `WORKLOAD_SVID_TTL / 2` (1800s, derived-from-TTL, not a bare literal);
   single-cut retired the `ROTATION_ENABLED` / `CERT_ROTATION_WORKFLOW` consts +
   `StartWorkflow` / `WorkflowName` imports; un-skipped the `near_expiry` `<=`
   boundary as a live mutation target (removed the `.cargo/mutants.toml`
   `exclude_re` entry) and landed the inclusive-`<=` boundary kill-test.
   Pure Tier-1 DST, no EDD.
2. **Slice ② — Boot-side #215 (persistent CA wired into `serve`)**
   (02-01..03): additive `ControlPlaneError::CaBoot(#[from] CaBootError)` +
   exhaustive `to_response` arm (no `Internal(String)` flatten); the **mandatory
   `ServerConfig.kek: Arc<dyn Kek>` injection seam** (C1-AMEND — `Default` impl
   removed, `ServerConfig::new(kek)` constructor added); `run_server` consumes
   `config.kek` into `boot_ca` + `bootstrap_node_intermediate`, single-cut
   replacing the ephemeral `RcgenCa` block; every `run_server` fixture (~26
   callers) injects the hermetic `SimKek::for_boot()` test double. **Captures
   EDD D01 + O04** (both `satisfied`, different-fox audited).
3. **Slice ③ — Consumer-side #215 (issued-cert summary + E03 proof)**
   (03-01..03): additive `IssuedCertSummary { serial, spiffe_id, issuer_serial,
   not_after }` + `AllocStatusResponse.issued_certificates` (skip-if-empty);
   server aggregates `issued_certificate_rows()` and projects the
   **max-`issuance_ordinal`** row per running alloc; `overdrive alloc status`
   renders it. The `issuance_ordinal` monotonic selection key (D1-AMEND, below)
   was minted here to fix the equal-`issued_at` tie. E03 runner extended from 2
   to **3 sub-claims** (chain verifies + leaf profile + pathLen=0 negative
   anchor) via an env-gated `OD_E03_CA_DIR` PEM export. **Captures EDD E03 +
   O05** (E03 `satisfied`, different-fox audited; **O05 `pending` — #227**).

## Key decisions (DESIGN D-OC-1..9 + two DELIVER-review amendments)

- **D-OC-1** — #40 near-expiry reissue is a reconciler **action**
  (`Action::IssueSvid` rotate-correlation via `SvidLifecycle`), **NOT a
  workflow**. A single internal mint+swap coordinates no ≥2 external steps and
  has no external-wait terminal, so it fails the workflow-candidacy test
  (`.claude/rules/workflows.md`). Reuses the existing variant UNCHANGED.
- **D-OC-2** — the near-expiry branch emits unconditionally; the
  `ROTATION_ENABLED` gate is retired (it existed only because the prior design
  routed rotation through an unregistered `cert_rotation` workflow that would
  raise `UnknownWorkflow` every tick).
- **D-OC-3** — near-expiry threshold = ½ × `WORKLOAD_SVID_TTL` (1800s),
  derived-from-TTL (persist-inputs spirit) so it tracks the policy const, not a
  bare literal. (Verified: `WORKLOAD_SVID_TTL` is 3600s, not the 24h the prior
  placeholder assumed.)
- **D-OC-4** — wire `ca_boot::boot_ca` + `bootstrap_node_intermediate` into
  `run_server` (`lib.rs:1595`), closing the D-CA-4 "CA not wired into serve"
  deferral. KEK-backed, envelope-sealed, refuse-to-start, adopt-on-restart — all
  already implemented; this feature only newly *calls* them at production boot.
- **D-OC-5** — `ControlPlaneError::CaBoot(#[from] CaBootError)` — a dedicated
  `#[from]` variant (never flatten a typed boot error to `Internal(String)`), so
  the composition root can `matches!` on `CaBoot(_)` and the distinct `CaError`
  cause (wrong-KEK vs tampered) survives to the operator.
- **D-OC-6** — restart = re-mint (confirm only): leaf keys are non-persistable
  (ADR-0063 D9), so the held set is empty on boot and the audit-row `ever_issued`
  signal drives immediate re-issue (ADR-0067 D10). No reshape.
- **D-OC-7** **(AMENDED — D1-AMEND)** — additive
  `AllocStatusResponse.issued_certificates`, projecting the
  **max-`issuance_ordinal`** row per running alloc (was latest-by-`issued_at`,
  which ties under a fixed `SimClock`).
- **D-OC-8** — un-skip the `near_expiry` mutation boundary (now a live, tested
  target).
- **D-OC-9 / C1-AMEND** (DELIVER-review amendment, 2026-06-10) — the `Kek`
  provider is injected through a **mandatory** `ServerConfig.kek: Arc<dyn Kek>`
  field (`ServerConfig: Default` removed; `ServerConfig::new(kek)` added);
  production composes `SystemdCredsKeyring::new()` at the CLI `serve` boundary,
  tests inject a hermetic `SimKek::for_boot()`. This replaces C1's original
  inline `SystemdCredsKeyring::new()` in `run_server`, which was the
  `.claude/rules/development.md` § "Port-trait dependencies" anti-pattern: the
  inline production binding forced a cold-environment `KekUnavailable` boot
  refusal on every test fixture (masked locally only by a leaked kernel-keyring
  key). The **mandatory** shape is the only one where a forgotten KEK is a
  *compile error* rather than a silent cold-boot failure — a defaulted `kek`
  (like `clock`) or an `Option`-override (like `dataplane_override`) both
  reproduce the regression for this trait.
- **D1-AMEND** (DELIVER-review amendment, 2026-06-11) — a monotonic
  `IssuanceOrdinal(u64)` newtype, sourced at the `issue_and_audit` seam as the
  count of already-persisted `issued_certificates` rows, is the deterministic,
  recency-correct "current cert" selection key. See § Lessons.

## Upstream doc corrections (back-propagation)

This feature was a net correction to three prior docs, all landed:

| Target | Change |
|---|---|
| ADR-0067 (rev 6→7) | A5 reframe (rotation = permanent reconciler *action*, not a throwaway sync-rotate); D8 + #40-boundary rewrite (emit `IssueSvid`, drop the wait-for-DNS-propagation workflow fiction); D1/D8 restart-re-mint note; rev 7 added: rotation participates in retry/backoff with a deadline-aware clamp. |
| ADR-0063 (dated amendment) | #215 wires `boot_ca` / `bootstrap_node_intermediate` into `run_server` (closes D-CA-4); records `ControlPlaneError::CaBoot` + O04 cause-distinctness; D01/O04 pending→wired. |
| `.claude/rules/workflows.md` § "Codebase precedent" | Corrected the "canonical first workflow = certificate rotation (#40) … wait for DNS propagation" claim — no first-party production workflow ships yet; internal SVID near-expiry reissue is a reconciler action; the wait-for-DNS-propagation 4-step shape is external-ACME, a separate Phase-5 concern. |
| `docs/product/architecture/brief.md` | Shipped — Component Inventory entry + dated changelog row (this finalize). |
| `.cargo/mutants.toml` | Removed the `"near_expiry"` `exclude_re` entry (the boundary is now a live mutation target). |

## Lessons & review findings

The headline implementation landed across 8 DES steps; two DELIVER adversarial
reviews then surfaced contract defects that became the two amendments above.

- **Inline production port-bindings are a cold-environment landmine (C1-AMEND).**
  The boot-wiring step's DELIVER review caught that C1's inline
  `SystemdCredsKeyring::new()` in `run_server` forced the production `Kek`
  binding onto every one of ~26 test fixtures. In a cold environment (no
  `$CREDENTIALS_DIRECTORY`, empty kernel keyring) that returns
  `KekUnavailable` → boot refuses → every fixture panics. It was masked locally
  by a leaked persistent kernel-keyring key of unknown provenance; on a fresh CI
  VM all ~26 fail identically. The fix made the `Kek` a **mandatory injected
  field** so a forgotten binding is a compile error, and replaced the masked
  local keyring with a hermetic `SimKek::for_boot()` in-process double. This is
  the exact `development.md` § "Port-trait dependencies" anti-pattern, caught in
  review rather than on a cold CI run. A companion correction landed at the same
  time: **a `--no-run` compile-check gate cannot see a cold-boot refusal** (a
  `--no-run` build never calls `boot_ca`), so a hook banning bare nextest
  `--no-run` was added, and the boot suite's gate became an actual Lima *run*.
- **Equal-`issued_at` ties surface a stale cert as "current" (D1-AMEND).**
  Step-0302 review (`review-03-02.md`, findings 1+2, verified against source)
  found the consumer's `max_by_key(|c| c.issued_at)` "current cert" projection is
  not strictly ordered: `issued_at` is a `SimClock` reading, and a fixed/seeded
  clock stamps two same-tick issuances identically. On a tie, `max_by_key`
  resolves by the audit store's serial-keyed iteration order — i.e. the largest
  CSPRNG-drawn *serial*, with no relation to recency — so a stale cert could
  render as current, falsifying S-OC-12. The fix added a monotonic
  `IssuanceOrdinal(u64)` (newtype, string-codec serde mirroring `CertSerial`),
  sourced as `issued_certificate_rows().len()` read at the `issue_and_audit`
  seam, and switched the projection to `max_by_key(issuance_ordinal)`. The two
  rejected alternatives were correctly rejected: enforce-unique-`issued_at` would
  *fabricate an audit fact*, and project-from-held-SVID-serial reads the *wrong
  SSOT* (the held set is empty on restart). The chosen source is honest, durable,
  and reads the audit SSOT.
- **The append-only precondition is the entire basis for the `len()`-derived
  ordinal — and it is load-bearing across a future phase.** The `count = len()`
  source is monotonic ONLY because `issued_certificates` rows are never
  deleted/overwritten/compacted. If a future phase adds a delete path
  (Phase-5 revocation pruning revoked certs, or an audit-log GC sweep), `len()`
  stops being monotonic and the ordinal becomes non-unique — directly
  reintroducing the equal-`issued_at` tie this amendment exists to fix. Whatever
  future work first adds a delete path MUST re-source the ordinal (a persisted
  monotonic counter a delete cannot rewind). This is **tracked, not deferred
  hand-wavily**, as
  [overdrive-sh/overdrive#226](https://github.com/overdrive-sh/overdrive/issues/226).
- **The stale "#40 = rotation workflow" framing was a conflation, now
  corrected.** ADR-0067 and GH #40 had described internal SVID rotation with the
  external-ACME `request → wait-for-DNS-propagation → validate → publish`
  four-step workflow shape. This feature established that internal SVID reissue
  is a single synchronous `Action::IssueSvid` — a reconciler action by the
  workflows.md decision rule — and back-propagated the correction into ADR-0067,
  `.claude/rules/workflows.md`, and the brief.

## Two finalize caveats (recorded honestly)

1. **O05 EDD evidence is `pending`, tracked → GH #227.** The operator-observable
   `issued_certificates` audit-row behavior IS implemented and integration-tested
   (`alloc_status_issued_certificates` drives the real handler projection), but
   the black-box EDD capture needs a live deploy → converge → issuance →
   `overdrive alloc status` path the in-process harness cannot run — it requires
   a disposable full-system VM ([#227]). D01, E03, O04 are `satisfied`
   (different-fox audited); **O05 stays `pending`** and is **not** marked
   satisfied. No new GH issue was created — #227 already tracks the capture
   blocker.
2. **rkyv schema-evolution judgment call (greenfield single-cut).** This feature
   appended `issuance_ordinal` to `IssuedCertificateRowV1` in place and
   regenerated `FIXTURE_V1` (discriminant offset 96→104, both sources re-pinned
   in one commit) rather than minting a `V2` envelope. This is precisely the move
   the golden-bytes schema-evolution test exists to catch — and it was a
   **deliberate, accepted decision** under the greenfield single-cut policy
   (`feedback_single_cut_greenfield_migrations.md`): V1 has **not** shipped to a
   deployed consumer (Phase 1, pre-GA, no persisted `issued_certificates` in the
   wild; "delete the on-disk redb file" is the official upgrade path), so V1 is
   still mutable and the field is part of the *initial* V1 shape, not a
   post-ship evolution. Minting `V2` would assert a version history that never
   existed. **Forward constraint:** if `issued_certificates` ever ships
   persistently, future field additions MUST mint `V2` (the fixture header
   comment and `development.md` § "rkyv schema evolution" both pin this).

## Quality gates (this finalize session — recorded, not re-run)

- **Implementation**: 8/8 steps COMMIT-traced (legacy 5-phase DES log); DES
  integrity exit 0.
- **Green baseline (Lima)**: compile clean · 1645 tests pass (14 skipped) ·
  doctests pass.
- **Phase 3 Refactor (L1–L6)**: one L3 dedup (`encode_framed_material` in
  `ca_boot.rs`); the rest already clean → `19e665eb`.
- **Phase 4 Adversarial review** (Opus, `@nw-software-crafter-reviewer`):
  **APPROVE, 0 blockers**; 7 dimensions verified clean (no invented API surface;
  two-CA distinction holds; rotation is a reconciler `Action::IssueSvid`, not a
  workflow; crypto-at-rest sound; no Testing Theater). 2 non-blocking doc
  findings fixed → `11363cde`.
- **Phase 5 Mutation** (diff-scoped, ≥80% gate): **PASS — 100% kill rate** (45
  mutants: 38 caught, 0 missed, 0 timeout, 7 unviable).
- **Phase 6 DES integrity**: PASS (exit 0).
- **Machine artifacts** committed → `e9711be5`.

## Pointers

- ADRs: `docs/product/architecture/adr-0067-workload-identity-manager-svid-lifecycle.md`
  (rev 7) · `docs/product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md`
  (amendment)
- Feature workspace (preserved as history):
  `docs/feature/built-in-ca-operator-composition/` — the lean v3.14
  `feature-delta.md` is the lasting design artifact (DISCUSS/DESIGN/DISTILL +
  DELIVER `[REF]` sections)
- Verification (EDD catalogue, operational SSOT — NOT migrated, stays in
  `verification/`): `verification/expectations/{D01,O04,E03}-*` (`satisfied`,
  different-fox audited) · `verification/expectations/O05-*` (`pending` — #227)
- Prior feature: [2026-06-06-built-in-ca](2026-06-06-built-in-ca.md) (GH #28 ·
  ADR-0063 · the X.509 hierarchy this composes)
- GH refs: **#215** (persistent boot-side composition — closed by this feature) ·
  **#40** (near-expiry rotation, reframed to a reconciler action) · **#28** (base
  built-in CA, prior feature) · **#226** (append-only precondition for the
  issuance-ordinal source — future delete-path constraint) · **#227** (O05
  capture blocker: disposable full-system VM)
- Deferrals (existing issues, cited not invented): multi-node audit gossip #36 ·
  SVID consumer (sockops mTLS/kTLS) #26 · revocation-coupled rotation Phase 5 /
  whitepaper §8
