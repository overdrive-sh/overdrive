# Feature Delta — `built-in-ca-operator-composition`

**Wave:** DESIGN · **Paradigm:** OOP Rust (CLAUDE.md) · **Scope:** Application ·
**Density:** lean (Tier-1 `[REF]` sections) · **Mode:** propose (guide-mode
rulings settled — see § Settled Decisions)

Compose the persistent built-in CA into the production `run_server` boot path
and complete/prove the workload-SVID lifecycle. **Folds GH #40 (near-expiry
rotation) + GH #215 (operator surface + EDD).** Builds on the already-shipped
`built-in-ca` (ADR-0063) and `workload-identity-manager` (ADR-0067) features —
this feature is *composition + lifecycle completion*, not new subsystem.

The single load-bearing reframe (back-propagated below): **#40 near-expiry
reissue is a reconciler ACTION (`Action::IssueSvid` rotate-correlation), not a
workflow.** Internal SVID near-expiry reissue does not coordinate ≥2 external
steps; it is a single internal mint+swap the executor already performs. The
"4-step wait-for-DNS-propagation workflow" framing in prior docs was external
ACME, never internal SVID reissue.

---

## [REF] Verified facts (settled at WRITE time)

| Fact | Verified value | Source |
|---|---|---|
| `WORKLOAD_SVID_TTL` | **`Duration::from_secs(3600)` — 1 hour** | `crates/overdrive-core/src/ca/validity.rs:30` |
| Near-expiry threshold (derived) | **½ × `WORKLOAD_SVID_TTL` = 1800s — 30 minutes** | SPIRE half-life norm (`docs/research/security/workload-svid-rotation-lifecycle-comprehensive-research.md`) |
| `CaError` wrong-KEK vs tampered split | **ALREADY SPLIT** — distinct variants `WrongKek` + `TamperedEnvelope`, each with a distinct `#[error(...)]` Display | `crates/overdrive-core/src/traits/ca.rs:546-607` |
| O04 sub-claim 2 (distinct messages) | **PASSES — no in-scope `CaError` fix needed** | `CaBootError::EnvelopeDecrypt { #[source] source: CaError }` preserves the distinct `CaError` Display via `cause: {source}` (`ca_boot.rs:84-89`) |

**Threshold-tracks-TTL discipline (persist-inputs spirit).** The near-expiry
threshold is derived as `WORKLOAD_SVID_TTL / 2`, NOT a bare literal — so it
tracks the TTL automatically if the policy changes. The current placeholder
const (`NEAR_EXPIRY_THRESHOLD_SECS = 28_800`, 8h) was wrong on two counts: it
assumed a 24h TTL (real TTL is 1h) and it was a bare literal not tied to the
TTL. Both are corrected here.

**`CaError` Display finding — NO in-scope fix.** ADR-0063 D3's claim that AEAD
distinguishes wrong-KEK from tampered-envelope is TRUE in code: the two are
separate `CaError` variants (`WrongKek { sealed_under, supplied }` /
`TamperedEnvelope { kek_id }`), each rendering a distinct operator-facing
message. `CaBootError::EnvelopeDecrypt` embeds the typed `CaError` as `#[source]`
and renders `cause: {source}`, so the boot stderr surfaces the distinct cause.
O04 sub-claim 2 is satisfiable as-is.

---

## [REF] DDD — subdomain / decision list

This feature touches one bounded context (workload identity / CA) and the
control-plane composition root. No new aggregates. DDD verdicts:

| D# | Decision | Verdict | Rationale |
|---|---|---|---|
| **D-OC-1** | #40 near-expiry reissue is a reconciler **action** (`Action::IssueSvid` rotate-correlation via `SvidLifecycle`), not a workflow | **Reconciler action** | Internal SVID reissue = single mint+swap the `IssueSvid` executor already does (`.claude/rules/workflows.md`: a workflow needs ≥2 coordinated external steps with a terminal result; this is neither). Reuses the existing `Action::IssueSvid` variant UNCHANGED. |
| **D-OC-2** | Near-expiry branch emits unconditionally (retire the `ROTATION_ENABLED` gate) | **Retire gate** | The gate existed only because the prior design routed rotation through an unregistered `cert_rotation` workflow (would raise `UnknownWorkflow` every tick). With the action reframe there is no workflow to register — the branch emits `IssueSvid` directly, always live. |
| **D-OC-3** | Near-expiry threshold = ½ × `WORKLOAD_SVID_TTL` (1800s) | **½ leaf TTL** | SPIRE half-life norm; derived-from-TTL so it tracks policy (persist-inputs spirit). |
| **D-OC-4** | #215 boot-side: flip `lib.rs:1595` ephemeral `RcgenCa::new` → `ca_boot::boot_ca` + `bootstrap_node_intermediate` | **Persistent boot** | Closes the D-CA-4 "CA not wired into serve" deferral. KEK-backed, envelope-sealed, Earned-Trust refuse-to-start, adopt-on-restart — all already implemented in `ca_boot.rs`; this wires it into `run_server`. |
| **D-OC-5** | `ControlPlaneError::CaBoot(#[from] CaBootError)` — distinct variant | **Dedicated variant** | `development.md` § Errors — never flatten a typed boot error to `Internal(String)`; the composition root must `matches!` on `CaBoot(_)` and the distinct `CaError` cause (wrong-KEK vs tampered) must survive to the operator. |
| **D-OC-6** | Restart = re-mint (no leaf keys at rest); #35's `running ∧ ¬held ∧ ever_issued → IssueSvid` branch is correct as-is | **Confirm only** | Leaf keys are non-persistable (ADR-0063 D9); on boot the held set is empty, the audit-row `ever_issued` signal drives immediate re-issue (ADR-0067 rev 5 D10). No reshape — re-validated against this feature. |
| **D-OC-7** | #215 consumer-side: additive `AllocStatusResponse.issued_certificates: Vec<…>`, latest-by-`issued_at` per running alloc | **Additive field** | Append-only audit ⇒ many rows per alloc over time; render the CURRENT cert (latest-by-`issued_at` matching `SpiffeId::for_allocation(...)`), NOT history, NOT cert bytes/keys (ADR-0067 #215-boundary). |
| **D-OC-8** | Un-skip the `near_expiry` mutation boundary | **Live mutation target** | With the gate retired the `<=` boundary is observable (a real `IssueSvid` emit), so it is a live mutation target needing a kill test — remove the `#[mutants::skip]`-equivalent `exclude_re` entry. |

---

## [REF] Component decomposition (paths + change type)

| Component | Path | Change type | What changes |
|---|---|---|---|
| `SvidLifecycle` reconciler | `crates/overdrive-core/src/reconcilers/svid_lifecycle.rs` | **MODIFY** | Delete `ROTATION_ENABLED` + `CERT_ROTATION_WORKFLOW` consts; delete `StartWorkflow` / `WorkflowName` imports; near-expiry branch emits `Action::IssueSvid { alloc_id, spiffe_id: held.spiffe_id.clone(), node_id: running.node_id.clone(), correlation: identity_correlation(alloc_id, &held.spiffe_id, "rotate-svid") }` unconditionally; set `NEAR_EXPIRY_THRESHOLD = WORKLOAD_SVID_TTL / 2`; remove `#[mutants::skip]` on `near_expiry`. **No new API surface** (reuse `Action::IssueSvid` unchanged). |
| `.cargo/mutants.toml` | `.cargo/mutants.toml:485-514` | **MODIFY** | Remove the `"near_expiry"` `exclude_re` entry + its comment block (the boundary is now a live, tested target). |
| `run_server` boot composition | `crates/overdrive-control-plane/src/lib.rs:1580-1600` | **MODIFY** | Replace ephemeral `RcgenCa::new` + `ca.root()` + `ca.issue_intermediate()` with: construct `SystemdCredsKeyring` + `RootKeyAeadCodec::new()` + `root_kek_id()`; coerce `store` to `Arc<dyn IntentStore>`; `boot_ca(...).await?` then `bootstrap_node_intermediate(...).await?`; build the bundle from the adopted CA; `.await` on both. |
| `ControlPlaneError` | `crates/overdrive-control-plane/src/error.rs:349-547` | **MODIFY** | Add `Ca`-boot variant `CaBoot(#[from] CaBootError)` + its exhaustive `to_response` arm (boot-path → 500, exhaustiveness-only). |
| `AllocStatusResponse` | `crates/overdrive-control-plane/src/api.rs:207-257` | **MODIFY** | Add additive `issued_certificates: Vec<IssuedCertSummary>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`); new `IssuedCertSummary { serial, spiffe_id, issuer_serial, not_after }` wire struct (NO cert bytes, NO key). |
| `alloc status` handler | `crates/overdrive-control-plane/src/` (alloc-status read path) | **MODIFY** | Aggregate `obs.issued_certificate_rows()`, project per running alloc the latest-by-`issued_at` row whose `spiffe_id == SpiffeId::for_allocation(workload_id, alloc_id)`, into `issued_certificates`. |
| CLI render | `crates/overdrive-cli/src/` (alloc-status render) | **MODIFY** | Render each `IssuedCertSummary` as `serial / spiffe_id / issuer_serial / not_after`. |
| `ca_boot` | `crates/overdrive-control-plane/src/ca_boot.rs` | **REUSE AS-IS** | `boot_ca` + `bootstrap_node_intermediate` already fully implemented (KEK probe, envelope decrypt, adopt-on-restart, `health.startup.refused`). No change — only newly *called*. |

---

## [REF] Driving ports (inbound)

| Port | Adapter | Notes |
|---|---|---|
| Operator CLI — `overdrive serve` | `overdrive-cli::commands::serve` → `run_server` | The boot composition root; now refuses to start with a structured `CaBoot` error on KEK-absent / envelope-decrypt-failure (O04). |
| Operator CLI — `overdrive job list` / `alloc status` | `overdrive-cli` alloc-status render | Now surfaces the current SVID summary per running alloc (O05/D1). |

No new driving ports. Both are existing CLI verbs gaining additional
observable surface.

---

## [REF] Driven ports + adapters (outbound) — Earned-Trust probes

| Port (trait) | Production adapter | Sim adapter | Probe contract (Earned Trust) |
|---|---|---|---|
| `Kek` | `SystemdCredsKeyring` (`overdrive-host`) | `SimKek` / fixture | **Probe (a):** `kek.resolve(kek_id)` MUST succeed before any issuance; absence → `CaBootError::KekUnavailable` + `health.startup.refused`, NO throwaway KEK. Already implemented in `boot_ca`. |
| `Ca` | `RcgenCa` (now persistent via `boot_ca` adopt) | `SimCa` / fixture | **Probe (b):** the persisted envelope MUST AES-GCM-open under the resolved KEK; tampered/wrong-KEK → `CaBootError::EnvelopeDecrypt` + `health.startup.refused`, NO silent re-mint (orphans every issued identity). Already implemented in `load_persistent_root` / `load_persistent_intermediate`. |
| `IntentStore` | `LocalIntentStore` (redb) | `SimIntentStore` | Root-key envelope + public cert material persisted/loaded; the boot path threads `redb_path` so the refuse-to-start remediation names the real file. The production type is `LocalIntentStore` (`crates/overdrive-store-local/src/redb_backend.rs`), opened at the control-plane composition root. |
| `ObservationStore` | `LocalObservationStore` (Corrosion/CR-SQLite) | `SimObservationStore` | `issued_certificate_rows()` is the append-only audit SSOT the consumer-side aggregates and the `ever_issued` restart signal reads. |

**Earned-Trust composition-root invariant: wire → probe → use.** `boot_ca`
already enforces it (KEK probe before generate/load; envelope decrypt-probe
before adopt). This feature's only Earned-Trust obligation is to **wire the
already-probing `boot_ca` into `run_server`** so the probes actually run at
production boot — the prior ephemeral path probed nothing. Fault-injection
scenarios the probes must survive (already covered by `ca_boot_and_audit.rs`
S-02-06/07, exercised in production via D-OC-4): tampered envelope, wrong KEK,
absent KEK.

---

## [REF] Technology choices (OSS-first, with rationale)

| Choice | License | Rationale |
|---|---|---|
| `ring` / `aws-lc-rs` (AES-GCM via `RootKeyAeadCodec`) | ISC / Apache-2.0 | Already in graph (ADR-0063); KEK-backed envelope seal/open. No new dep. |
| `rcgen` (`RcgenCa`) | MIT/Apache-2.0 | Already in graph; the `Ca` production adapter. No new dep. |
| `redb` (`IntentStore`) | MIT/Apache-2.0 | Already in graph; root-key envelope + cert-material persistence. No new dep. |
| systemd-creds / kernel keyring (`SystemdCredsKeyring`) | OS-provided | Already implemented (ADR-0063 D3/D6); the production KEK provider. No new dep. |

**No new dependencies.** Every technology this feature composes already ships
in the workspace from `built-in-ca` (ADR-0063) and `workload-identity-manager`
(ADR-0067). This is a *composition* feature.

---

## [REF] Decisions table (with back-propagation)

| Ruling | Shape (exact) |
|---|---|
| **A1** rotate emit | `Action::IssueSvid { alloc_id: alloc_id.clone(), spiffe_id: held.spiffe_id.clone(), node_id: running.node_id.clone(), correlation: identity_correlation(alloc_id, &held.spiffe_id, "rotate-svid") }` — reuse the existing variant, NO new field/flag. `running` (the `RunningAlloc`) is in scope in the `running ∧ held` arm; source `node_id` from `running.node_id` (no `HeldSvidFacts` change). |
| **B1** gate + threshold | Delete `ROTATION_ENABLED` + `CERT_ROTATION_WORKFLOW` consts + `StartWorkflow`/`WorkflowName` imports; near-expiry emits `IssueSvid` unconditionally; `NEAR_EXPIRY_THRESHOLD = WORKLOAD_SVID_TTL / 2` (1800s). Remove `#[mutants::skip]` on `near_expiry` + the `.cargo/mutants.toml` `exclude_re` entry; the `<=` boundary is a live mutation target → kill-test DELIVER obligation. |
| **C1** boot wiring | At `lib.rs:1595`: construct `SystemdCredsKeyring::new()` + `RootKeyAeadCodec::new()` + `root_kek_id()`; `let intent: Arc<dyn IntentStore> = Arc::clone(&store) as Arc<dyn IntentStore>;`; `let root = boot_ca(ca.as_ref(), &kek, &kek_id, &codec, &intent, &store_path).await?;` then `bootstrap_node_intermediate(ca.as_ref(), &node_id, &intent, &kek, &kek_id, &codec, &store_path).await?`; build `trust_bundle()` from the adopted CA → `IdentityMgr`. Add `ControlPlaneError::CaBoot(#[from] CaBootError)`. |
| **D1** consumer field | Additive `AllocStatusResponse.issued_certificates: Vec<IssuedCertSummary>` (skip-if-empty); server aggregates `issued_certificate_rows()` and projects per running alloc the latest-by-`issued_at` row whose `spiffe_id == SpiffeId::for_allocation(...)`; CLI renders `serial / spiffe_id / issuer_serial / not_after` — NO cert bytes, NO key. |
| **E1** slices + EDD | ONE feature-delta, 3 DELIVER slices (below); EDD per slice via `verification/harness/run-expectation.sh` in Lima, then a different-fox Haiku reviewer PER expectation over `evidence/` before any `satisfied`. |

---

## [REF] Reuse Analysis (HARD GATE)

| Candidate | Verdict | Evidence |
|---|---|---|
| `ca_boot::boot_ca` + `bootstrap_node_intermediate` | **REUSE AS-IS — only newly called** | Fully implemented in `ca_boot.rs` (KEK probe (a), envelope decrypt-probe (b), generate-or-load, adopt-on-restart, `health.startup.refused`, redb_path-threaded remediation). This feature *calls* it from `run_server` — no signature change, no logic change. Closes D-CA-4. |
| `SystemdCredsKeyring` (`Kek`) | **REUSE AS-IS** | Production KEK provider (ADR-0063 D3/D6); `SystemdCredsKeyring::new()` reads `$CREDENTIALS_DIRECTORY` at resolve time. Constructed at the boot root. No change. |
| `RootKeyAeadCodec` | **REUSE AS-IS** | `RootKeyAeadCodec::new()` over the crypto-backend CSPRNG; `seal`/`open`/`seal_intermediate`. No change. |
| `root_kek_id()` | **REUSE AS-IS** | `KekId::new("overdrive-ca-root")` — the stable single-node KEK identity. No change. |
| `Action::IssueSvid` | **REUSE AS-IS (UNCHANGED)** | The rotate path reuses the existing variant with a `"rotate-svid"` correlation purpose — NO new field/flag/variant (honors "never invent API surface"). The `node_id` comes from `running.node_id` already in scope. |
| `identity_correlation(alloc, spiffe_id, purpose)` | **REUSE AS-IS** | Already derives `CorrelationKey` for `"issue-svid"`/`"drop-svid"`; the rotate path passes `"rotate-svid"` as a third purpose value (a string arg, not new API). No change. |
| `SvidLifecycle` reconciler runtime + ViewStore | **REUSE AS-IS** | The reconciler stays one `Reconciler` on the shipped runtime; only its `reconcile` body's near-expiry branch changes (gate retired, emit flipped). No runtime change. |
| `IssueSvid` executor (action-shim) | **REUSE AS-IS** | The rotate-correlation `IssueSvid` dispatches through the SAME executor (`action_shim/issue_svid.rs`) that first-issue/restart-reissue use — `issue_and_audit` mints + audits + the holder `hold`-replaces. No executor change. |
| `IssuedCertificateRow` | **REUSE AS-IS** | The consumer-side projects `serial / spiffe_id / issuer_serial / not_after / issued_at` from existing fields. No row-schema change (no rkyv envelope bump). |
| `ObservationStore::issued_certificate_rows()` | **REUSE AS-IS** | Existing append-only read surface; both the consumer aggregation and the `ever_issued` restart signal already read it. No change. |
| `SpiffeId::for_allocation(&WorkloadId, &AllocationId)` | **REUSE AS-IS** | Already the canonical derivation (ADR-0067 D5); the consumer matches audit rows on it. No change. |
| `AllocStatusResponse` | **EXTEND (additive)** | +1 `Vec<IssuedCertSummary>` field (skip-if-empty) + 1 new wire struct. Additive — existing consumers untouched, JSON backward-compatible. |
| `ControlPlaneError` | **EXTEND (additive)** | +1 `CaBoot(#[from] CaBootError)` variant + 1 exhaustive `to_response` arm. Additive — no existing variant changes. |
| `RcgenCa` ephemeral composition (`lib.rs:1580-1600`) | **DELETE (single-cut)** | The ephemeral `RcgenCa::new` + `root()` + `issue_intermediate()` block is replaced by `boot_ca` + `bootstrap_node_intermediate` in the same commit (single-cut greenfield — no parallel path, no flag). |
| `ROTATION_ENABLED` / `CERT_ROTATION_WORKFLOW` consts + `near_expiry` `#[mutants::skip]` + `mutants.toml` `exclude_re` | **DELETE (single-cut)** | The gate + workflow-name + the mutation suppression are all retired together with the action reframe — removed code AND its mutation exclusion in the same commit. |

**Verdict: 11 REUSE AS-IS, 2 EXTEND (additive — `AllocStatusResponse`,
`ControlPlaneError`), 3 DELETE (single-cut — ephemeral `RcgenCa` composition,
rotation gate consts, mutation exclusion).** Zero CREATE-NEW beyond one additive
wire struct (`IssuedCertSummary`). The profile confirms this is a composition +
lifecycle-completion feature: every CA / boot / audit / reconciler primitive
already exists; the work is *calling `boot_ca`*, *flipping one reconciler branch
from gated-workflow to direct-action*, and *projecting an existing audit row to
the operator*.

---

## DELIVER slices (3) + EDD capture plan

### Slice ① — Rotation (core action flip)

**Scope:** `svid_lifecycle.rs` near-expiry branch + gate retirement + mutation
un-skip.

- Delete `ROTATION_ENABLED`, `CERT_ROTATION_WORKFLOW` consts + `StartWorkflow` /
  `WorkflowName` imports.
- Near-expiry branch emits `Action::IssueSvid { ... correlation:
  identity_correlation(alloc_id, &held.spiffe_id, "rotate-svid") }`
  unconditionally (reuse the variant; `node_id` from `running.node_id`).
- `NEAR_EXPIRY_THRESHOLD = WORKLOAD_SVID_TTL / 2` (1800s, derived).
- Remove `#[mutants::skip]` on `near_expiry` AND the `.cargo/mutants.toml`
  `"near_expiry"` `exclude_re` entry.
- **DST test:** near-expiry held alloc → exactly one `IssueSvid` (rotate
  correlation) emitted when `held.not_after <= now + 1800s`; none otherwise.
- **Mutation kill-test (DELIVER obligation):** the `near_expiry` `<=` boundary
  is now live — a `<=`→`<` / `<=`→`==` flip must be killed by the boundary DST
  test. The un-skipped helper is a mandatory mutation target.
- **No EDD** (pure in-process reconciler logic; stays in the test tiers).

### Slice ② — Boot-side #215 (persistent CA wired into `serve`)

**Scope:** `lib.rs:1580-1600` composition + `ControlPlaneError::CaBoot`.

- Construct `SystemdCredsKeyring::new()` + `RootKeyAeadCodec::new()` +
  `root_kek_id()`; coerce `store` → `Arc<dyn IntentStore>`.
- Replace ephemeral `RcgenCa` block: `boot_ca(...).await?` then
  `bootstrap_node_intermediate(...).await?`; build bundle from adopted CA.
- Add `ControlPlaneError::CaBoot(#[from] CaBootError)` + exhaustive `to_response`
  arm (boot-path → 500, exhaustiveness-only).
- **Captures EDD: D01** (root key never plaintext at rest — the persisted
  envelope is KEK-sealed) **+ O04** (refuse-to-start on decrypt failure with
  distinct wrong-KEK / tampered messages — the `CaBootError::EnvelopeDecrypt`
  `cause: {source}` surfaces the distinct `CaError`).
- **O04 cause-distinctness:** verified satisfiable — `CaError::WrongKek` and
  `CaError::TamperedEnvelope` are distinct Display messages; no `CaError` fix
  needed.

### Slice ③ — Consumer-side #215 (issued-cert summary surfaced)

**Scope:** `AllocStatusResponse.issued_certificates` field + server aggregation
+ CLI render.

- Add `IssuedCertSummary { serial, spiffe_id, issuer_serial, not_after }` +
  additive `AllocStatusResponse.issued_certificates: Vec<IssuedCertSummary>`
  (skip-if-empty).
- Server aggregates `obs.issued_certificate_rows()`, projects per running alloc
  the latest-by-`issued_at` row matching `SpiffeId::for_allocation(...)`.
- CLI renders `serial / spiffe_id / issuer_serial / not_after` (NO cert bytes,
  NO key).
- **Captures EDD: O05** (issued-certificates audit row is operator-visible; the
  current serial is legible). **E03 is captured by the separate exported-PEM
  `openssl verify` path below — NOT by the summary render.**

**O05 ≠ E03 — distinct surfaces, distinct proofs.** `issued_certificates` /
`IssuedCertSummary { serial, spiffe_id, issuer_serial, not_after }` is the
**O05 surface ONLY**. It deliberately carries **no cert bytes and no key**, so
it CANNOT prove chain verification — a `serial` and a `not_after` are
operator-legible metadata, not a verifiable certificate. The summary render
lets the operator *cross-check which cert is current*; it does not, and is not
intended to, satisfy E03. Do NOT imply that rendering the summary proves the
chain verifies.

#### E03 evidence path (test/EDD-capture only — NO production surface)

E03 requires `openssl verify -CAfile root.pem -untrusted intermediate.pem
svid.pem` (sub-claim 1) + leaf-profile checks (sub-claim 2: exactly one
`spiffe://` URI SAN, `CA:FALSE`, critical `digitalSignature`) + the
pathLen=0 negative anchor (sub-claim 3, S-03-05) over **actual PEM material**
at the runner's `$CA_DIR` (default `/tmp/od-e03-ca/{root,intermediate,svid}.pem`).
None of that surface exists in `IssuedCertSummary`. This slice OWNS the path
that puts those three PEMs where the runner expects them, with **no new
production API surface and no operator verb** that mints/exports a chain
(D-CA-4 / the E03 README both hold: SVID issuance stays an internal platform
mechanism this phase; `openssl verify` is the honest external entry point):

- **Export hook (test-only).** The already-gated `integration-tests` test
  `crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs::rcgen_full_svid_chain_verifies_root_intermediate_svid`
  ALREADY mints a coherent root → intermediate → SVID chain in-tree (same
  `RcgenCa` instance, real `ring`/rcgen crypto) and verifies it with `openssl`
  against a `tempfile::TempDir`. Add an **env-gated export** to it: when
  `$OD_E03_CA_DIR` is set, the test ALSO writes its three PEMs
  (`root.cert_pem()`, `inter.cert_pem()`, `svid.cert_pem()`) to that directory
  before the tempdir is dropped. The PEMs are emitted by the *existing* `Ca`
  port surface — root + intermediate from the minted chain (equivalently
  `Ca::trust_bundle()` → `root_anchor()` / `intermediate_chain()`, both
  PEM-encoded `CaCertPem`), the leaf from the minted `SvidMaterial::cert_pem()`.
  This is a test fixture change, not a library/CLI change — `IssuedCertSummary`
  and every production type are untouched.
- **Runner wiring (MUST enforce ALL THREE sub-claims before any `satisfied`).**
  Update `verification/expectations/E03-ca-full-chain-verifies/runner.sh`
  to run the gated test in Lima with `OD_E03_CA_DIR="$CA_DIR"` (e.g.
  `in_lima env OD_E03_CA_DIR="$CA_DIR" cargo nextest run -p overdrive-host
  --features integration-tests -E 'test(rcgen_full_svid_chain_verifies_root_intermediate_svid)'`),
  then run **sub-claim 1** (chain `openssl verify` → OK) and **sub-claim 2**
  (leaf profile: exactly one `spiffe://` URI SAN, `CA:FALSE`, critical
  `digitalSignature`) over the exported `$CA_DIR/{root,intermediate,svid}.pem`
  as the current runner already does. The current runner enforces ONLY
  sub-claims 1–2 and then exits — Slice ③ MUST also ADD **sub-claim 3** (the
  pathLen=0 negative anchor, S-03-05): a chain where the pathLen=0 intermediate
  signs a *further CA* MUST FAIL `openssl verify` (pathLen *enforced*, not
  merely *set*). Its source is named: under the same `OD_E03_CA_DIR` env-gate,
  export the further-CA chain from the existing test
  `crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs::rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced`
  and assert `openssl verify` **FAILS** on it (or capture that test's own
  failing-verification evidence). Flip the runner guard once the PEMs are
  present; the `pending` branch (no PEMs at `$CA_DIR`) stays as the honest
  fallback.

  **E03 is NOT satisfiable until the runner enforces sub-claims 1–3.** A
  two-check runner that goes green on chain-verifies + leaf-profile alone does
  NOT satisfy E03 — the negative anchor is the proof that pathLen is *enforced*,
  not decorative, and omitting it leaves the headline walking-skeleton claim
  half-proven. A DELIVER agent MUST NOT mark E03 `satisfied` against the present
  2-check runner; it MUST first extend the runner to the 3-check shape above,
  re-capture, and only then submit for the different-fox audit. The
  different-fox Haiku reviewer (below) MUST reject E03 evidence that omits the
  sub-claim-3 negative anchor.
- **Why this is sound (no fork).** The trust-bundle surface DOES yield
  PEM-encoded root + intermediate (`TrustBundle::root_anchor()` /
  `intermediate_chain()` are `CaCertPem`), and the leaf PEM is on
  `SvidMaterial::cert_pem()` — both already exist, so there is no
  "can't yield PEM" blocker. The chain is coherent because all three PEMs come
  from the SAME `RcgenCa` instance in one test, exactly the shape `openssl
  verify` requires. No dedicated E03 slice is needed: the in-tree mint already
  lives in the gated test the E03 README anchors on (S-04-07), so the
  in-Slice-③ export-and-capture is the minimal, honest path. (Surfaced
  fork — NONE: a dedicated E03 slice would only relocate the same test-only
  export hook + runner wiring with no added proof.)

### EDD capture discipline (all slices)

Each EDD expectation is captured via `verification/harness/run-expectation.sh
<ID>` inside Lima, SHA-pinned. D01 / O04 / O05 capture against the **built
`overdrive` binary** (operator-CLI / data-at-rest surfaces). **E03 is the one
exception:** there is no operator verb that mints/exports a chain this phase
(D-CA-4), so E03's runner produces its three PEMs as a **side-effect of the
gated `rcgen_ca_chain_verify.rs` integration test** (`OD_E03_CA_DIR` env-gated
export), then does its black-box proof — `openssl verify` + leaf-profile checks
over the exported `$CA_DIR/*.pem` files. **This stays within the
black-box discipline** (`verification/README.md`): the runner remains a bash +
`openssl` + file-observation surface and does NOT import or link any
`overdrive-*` crate — `cargo nextest` is invoked only as the *producer* of the
PEM artifacts, exactly the "gated integration test writes the PEMs to a known
dir" unblock path the E03 README/runner already sanction. Status is set to
`satisfied` ONLY after a **different-fox Haiku reviewer per expectation** reads
the captured `evidence/` adversarially ("refute that this evidence satisfies the
expectation; default to refuted if narrated rather than executed"). The
authoring agent never self-stamps `satisfied`. **For E03 specifically the audit
MUST verify all three sub-claims are present in the evidence — chain verifies
(1), leaf profile (2), AND the pathLen=0 negative anchor that FAILS `openssl
verify` (3, S-03-05). E03 evidence missing sub-claim 3 is a mandatory `refuted`,
even if sub-claims 1–2 executed cleanly:** the negative anchor is what proves
pathLen is enforced rather than merely set, so a 2-check capture is incomplete by
construction.

| Expectation | Slice | Surface | Proof mechanism |
|---|---|---|---|
| D01 — root key never plaintext at rest | ② | D (data-at-rest) | persisted envelope is KEK-sealed; no plaintext root key on disk |
| O04 — refuse-to-start, actionable error | ② | O (operator CLI) | boot refuses on decrypt failure; distinct wrong-KEK / tampered Display |
| E03 — full chain verifies | ③ | E (end-to-end) | **exported-PEM `openssl verify`** over `$CA_DIR/{root,intermediate,svid}.pem` (test-only env-gated export from `rcgen_ca_chain_verify.rs`; NOT the summary render). Runner MUST enforce ALL THREE sub-claims before `satisfied`: (1) chain verifies, (2) leaf profile, (3) pathLen=0 negative anchor FAILS `openssl verify` (S-03-05, sourced from `rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced`) |
| O05 — issued-certificates audit row | ③ | O (operator CLI) | `issued_certificates` summary render — operator-legible metadata, NOT a chain proof |

---

## Architecture Enforcement

Style: Hexagonal (ports-and-adapters) — already established (ADR-0063 `Ca` port,
ADR-0067 `IdentityRead` port; sim/host split per `CLAUDE.md` crate taxonomy).
Language: Rust. Tool: `xtask dst-lint` (the project's import-graph + AST gate).

Rules to enforce (already enforced; this feature must not regress them):
- `overdrive-core` (`core` class) takes no real-infra calls — the CA boot wiring
  lives in `overdrive-control-plane` (composes host adapters at the binary
  boundary), never in `core`.
- `overdrive-host` (`SystemdCredsKeyring`, `RootKeyAeadCodec`, `RcgenCa`) is the
  production binding, composed only at `run_server`.
- The `Ca` / `Kek` / `IntentStore` / `ObservationStore` port traits are the
  boundary; `boot_ca` takes `&dyn Ca` / `&dyn Kek` / `&Arc<dyn IntentStore>`.

---

## Open questions

None blocking. The two prior-deferred open questions are now resolved by this
feature:

- **`WORKLOAD_SVID_TTL` / near-expiry threshold** — RESOLVED: threshold = ½ ×
  `WORKLOAD_SVID_TTL` (1800s), derived-from-TTL.
- **#40 rotation primitive** — RESOLVED (reframe): near-expiry reissue is a
  reconciler action, not a workflow; no workflow primitive (#39) dependency.

---

## Changed Assumptions

> **Original (ADR-0067 D8, A5, #40-boundary; `.claude/rules/workflows.md`
> precedent):** "#40 owns the durable rotation *workflow* — the near-expiry →
> request → wait-for-DNS-propagation → validate → publish sequence (the textbook
> Bar-2 workflow). The near-expiry branch emits `Action::StartWorkflow(cert_rotation)`,
> gated behind `ROTATION_ENABLED` until #40 registers the workflow."

> **Replacement (this feature, D-OC-1/D-OC-2):** Internal SVID near-expiry
> reissue is a reconciler **action** — `Action::IssueSvid` with a `"rotate-svid"`
> correlation — emitted unconditionally by `SvidLifecycle::reconcile`. It is NOT
> a workflow: a single internal mint+swap does not coordinate ≥2 external steps
> and has no external-wait terminal. The `ROTATION_ENABLED` gate and the
> `cert_rotation` workflow name are DELETED. The "wait-for-DNS-propagation"
> 4-step shape was external-ACME public-cert rotation, never internal SVID
> reissue. External-ACME rotation (if it ever ships) remains a separate concern
> and would be the TBD candidate first production workflow (Phase 5,
> revocation-coupled, once it coordinates ≥2 external steps).

This back-propagates into ADR-0067 (rev 6: A5 reframe, D8 + #40-boundary
rewrite, D1/D8 restart note), ADR-0063 (dated amendment: D-CA-4 closure,
`ControlPlaneError::CaBoot`, D01/O04 pending→wired), and
`.claude/rules/workflows.md` § "Codebase precedent" (correct the "canonical
first workflow = certificate rotation" claim).
