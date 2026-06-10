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
| `run_server` boot composition | `crates/overdrive-control-plane/src/lib.rs:1580-1617` | **MODIFY** | Replace ephemeral `RcgenCa::new` + `ca.root()` + `ca.issue_intermediate()` with the persistent boot, consuming the **injected** `Kek` provider from `config.kek` (the C1-AMEND seam below) instead of constructing `SystemdCredsKeyring::new()` inline: `RootKeyAeadCodec::new()` + `root_kek_id()`; coerce `store` to `Arc<dyn IntentStore>`; `boot_ca(ca.as_ref(), config.kek.as_ref(), &kek_id, &codec, &intent_store, &store_path).await?` then `bootstrap_node_intermediate(...)` (same `config.kek.as_ref()`); build the bundle from the adopted CA. `boot_ca` / `bootstrap_node_intermediate` already take `&dyn Kek` — REUSE-AS-IS; only the *source* of the `&dyn Kek` changes (inline production binding → injected `config.kek`). |
| `ServerConfig` (C1-AMEND seam) | `crates/overdrive-control-plane/src/lib.rs:525-687 (struct), 715-773 (Default impl)` | **MODIFY** | Add the **mandatory** `Kek` injection seam — see § C1-AMEND below. New field `pub kek: Arc<dyn overdrive_core::ca::kek::Kek>`. `ServerConfig: Default` is **removed** (a mandatory `Arc<dyn Kek>` cannot be defaulted to a benign value — defaulting it to `SystemdCredsKeyring::new()` is the regression). A new `ServerConfig::new(kek: Arc<dyn Kek>) -> Self` constructor supplies every *other* field's former-`Default` value, so fixtures write `ServerConfig { ..ServerConfig::new(test_kek()) }` in place of `..Default::default()`. |
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
| `Kek` | `SystemdCredsKeyring` (`overdrive-host`) | `SimKek` (`overdrive-sim`) | **Probe (a):** `kek.resolve(kek_id)` MUST succeed before any issuance; absence → `CaBootError::KekUnavailable` + `health.startup.refused`, NO throwaway KEK. Already implemented in `boot_ca`. The hermetic test KEK injected by every `run_server` fixture is `overdrive_sim::adapters::SimKek::for_boot()` — a pure in-process `Kek` double (see § C1-AMEND C-3). |
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
| **C1** boot wiring **(AMENDED 2026-06-10 — see § C1-AMEND)** | At `lib.rs:1595`: consume the **injected** `Kek` from `config.kek` (do NOT construct `SystemdCredsKeyring::new()` inline); `let codec = RootKeyAeadCodec::new();` + `let kek_id = root_kek_id();`; `let intent: Arc<dyn IntentStore> = Arc::clone(&store) as Arc<dyn IntentStore>;`; `let root = boot_ca(ca.as_ref(), config.kek.as_ref(), &kek_id, &codec, &intent, &store_path).await?;` then `bootstrap_node_intermediate(ca.as_ref(), &node_id, &intent, config.kek.as_ref(), &kek_id, &codec, &store_path).await?`; build `trust_bundle()` from the adopted CA → `IdentityMgr`. Add `ControlPlaneError::CaBoot(#[from] CaBootError)`. The `Kek` provider is injected through `ServerConfig.kek` (mandatory field) — production composes `SystemdCredsKeyring::new()` at the CLI `serve` boundary; tests inject a hermetic fixture KEK. |
| **D1** consumer field | Additive `AllocStatusResponse.issued_certificates: Vec<IssuedCertSummary>` (skip-if-empty); server aggregates `issued_certificate_rows()` and projects per running alloc the latest-by-`issued_at` row whose `spiffe_id == SpiffeId::for_allocation(...)`; CLI renders `serial / spiffe_id / issuer_serial / not_after` — NO cert bytes, NO key. |
| **E1** slices + EDD | ONE feature-delta, 3 DELIVER slices (below); EDD per slice via `verification/harness/run-expectation.sh` in Lima, then a different-fox Haiku reviewer PER expectation over `evidence/` before any `satisfied`. |

---

## [REF] Reuse Analysis (HARD GATE)

| Candidate | Verdict | Evidence |
|---|---|---|
| `ca_boot::boot_ca` + `bootstrap_node_intermediate` | **REUSE AS-IS — only newly called** | Fully implemented in `ca_boot.rs` (KEK probe (a), envelope decrypt-probe (b), generate-or-load, adopt-on-restart, `health.startup.refused`, redb_path-threaded remediation). This feature *calls* it from `run_server` — no signature change, no logic change. Closes D-CA-4. |
| `SystemdCredsKeyring` (`Kek`) | **REUSE AS-IS** | Production KEK provider (ADR-0063 D3/D6); `SystemdCredsKeyring::new()` reads `$CREDENTIALS_DIRECTORY` at resolve time. **Composed at the CLI `serve` boundary and injected into `ServerConfig.kek`** (C1-AMEND) — NOT constructed inline inside `run_server`. The adapter type itself is unchanged; only its composition site moves outward to the binary boundary so tests inject a hermetic fixture KEK instead of inheriting the production binding. |
| `SimKek` (`overdrive-sim`, hermetic test `Kek` double) | **REUSE AS-IS** | `overdrive_sim::adapters::SimKek::for_boot()` — a pure in-process `Kek` test double (`crates/overdrive-sim/src/adapters/kek.rs`; preloads the canonical `overdrive-ca-root` KEK from a `BTreeMap`, no kernel keyring, no `$CREDENTIALS_DIRECTORY`, no FFI) — is the **hermetic test KEK** injected through `ServerConfig.kek` in every `run_server` fixture. A `Kek` fixture is a pure in-process double, so per `.claude/rules/development.md` § "Shared real-infra test fixtures" it belongs with the `Sim*` adapters in `overdrive-sim` (the sim/host split), NOT in `overdrive-testing`. Both consuming crates already dev-dep `overdrive-sim`, so zero new wiring. No adapter change. |
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

## § C1-AMEND — `Kek` injection seam (DELIVER-review amendment to C1, 2026-06-10)

**Why this amendment exists.** The originally-pinned C1 said "construct
`SystemdCredsKeyring::new()` … in `run_server`" — i.e. hardcode the production
`Kek` binding inline at the composition root, with no injection seam. That is
the exact anti-pattern `.claude/rules/development.md` § "Port-trait dependencies"
forbids: *"Never default the field to a production binding … that silently
inherits … behaviour into tests that forgot to override."* `Kek`
(`overdrive_core::ca::kek::Kek`) is a port trait; `boot_ca` already takes
`&dyn Kek`. With the inline binding, **every** fixture that boots through
`run_server` / `run_server_with_obs_and_driver` (~26 callers across
`tests/integration/` + `tests/acceptance/`) hits
`SystemdCredsKeyring::new().resolve("overdrive-ca-root")`, which in a **cold
environment** (no `$CREDENTIALS_DIRECTORY`, no `OVERDRIVE_CA_KEK` opt-in, empty
kernel keyring) returns `KekError::NotFound` → `CaBootError::KekUnavailable` →
boot refuses → the fixture panics at `.expect("run_server")`. This was masked
locally by a leaked persistent kernel-keyring key; on a fresh CI VM all ~26
callers fail identically (hard repro: `server_lifecycle.rs:105:65 … CaBoot(
KekUnavailable { kek_id: "overdrive-ca-root", source: NotFound { … } })`).

**The pinned shape (Option A, user-approved). The `Kek` is a MANDATORY injected
field — no `Default`, no `Option`-override.**

### Seam decision — why mandatory, not defaulted and not optional-override

`ServerConfig` carries both idioms today, and BOTH reproduce the regression for
this trait:

- A **defaulted** `kek` (mirroring `clock`'s `Arc::new(SystemClock)` default) is
  wrong because `clock`'s forgotten default is *benign* (a test silently uses
  wall-clock — a smell, but it boots), whereas `kek`'s forgotten production
  default `SystemdCredsKeyring::new()` is *malign* — it refuses to boot cold.
  The compiler does NOT catch the omission. This is precisely the
  "tests can forget" failure development.md warns against, and it is the exact
  defect just observed.
- An **`Option<Arc<dyn Kek>>` override** (mirroring `dataplane_override`,
  `None → SystemdCredsKeyring::new()` composed in `run_server`) is the SAME
  hazard spelled with `Option`: a `..Default::default()` fixture that forgets
  the override silently gets the cold-failing production KEK. development.md
  explicitly names optional/builder overrides an anti-pattern *for port traits*
  ("optional means tests can forget"). Reproduces the regression.
- A **mandatory** `kek: Arc<dyn Kek>` (development.md's stated preference) is the
  ONLY shape where a fixture that forgets the KEK **fails to compile** instead of
  failing cold at boot. The compiler — not a CI VM's keyring state — enforces
  that every boot site is explicit about its KEK.

**Churn is identical across all three shapes, so it is not a tie-breaker.** Every
fixture already constructs `ServerConfig { <required>, ..Default::default() }`
with an explicit struct literal (verified — e.g.
`server_lifecycle.rs:86-103`). Under the `Option` shape each forgetting-fixture
would add one line (`kek_override: Some(test_kek())`); under the mandatory shape
each adds one line (`kek: test_kek()`). Same one-line edit — but the mandatory
shape makes the omission a compile error and the `Option` shape makes it a
silent cold-boot failure. Given equal churn, the safer shape wins.

**`Default` consequence (accepted churn, not avoidable).** `..Default::default()`
requires `Self: Default`, which requires every field defaultable — so a truly
mandatory `kek` field forces **removing the `ServerConfig: Default` impl**.
There is no honest way to keep `Default` AND make `kek` mandatory (defaulting it
to a benign `Kek` would be a second hidden production-or-fake binding — the same
hazard). The resolution that preserves ergonomics for every *other* field is a
constructor:

```rust
impl ServerConfig {
    /// Construct a `ServerConfig` with the mandatory `kek` provider and every
    /// other field set to its prior `Default` value. Replaces the removed
    /// `Default` impl: the `Kek` port binding MUST be supplied explicitly
    /// (production composes `SystemdCredsKeyring::new()`; tests inject a
    /// hermetic fixture KEK) so a boot site that forgets it fails to COMPILE,
    /// never inherits the production binding and refuses to start in a cold
    /// environment. See feature-delta § C1-AMEND.
    #[must_use]
    pub fn new(kek: Arc<dyn overdrive_core::ca::kek::Kek>) -> Self {
        Self {
            kek,
            // … every field that the removed `Default::default()` body set,
            // moved verbatim into this constructor body …
        }
    }
}
```

Fixtures change `..Default::default()` → `..ServerConfig::new(test_kek())` (a
mechanical one-token swap per call site, plus the shared `test_kek()` helper).
The `#[cfg(feature = "integration-tests")] dataplane_probe_fault` field stays
cfg-gated inside the constructor body exactly as it is inside the current
`Default` body.

### Exact pinned signatures

| Element | Exact shape |
|---|---|
| New field | `pub kek: Arc<dyn overdrive_core::ca::kek::Kek>,` on `ServerConfig` (place adjacent to `clock`, with a rustdoc block stating: production composes `SystemdCredsKeyring::new()` at the CLI `serve` boundary; tests inject a hermetic `overdrive_sim::adapters::SimKek::for_boot()`; the field is mandatory specifically so a forgotten KEK is a compile error, not a cold-boot refusal — citing development.md § "Port-trait dependencies" and this § C1-AMEND). |
| `Default` impl | **REMOVED** (`impl Default for ServerConfig`, `lib.rs:715-773`). |
| Constructor | `pub fn ServerConfig::new(kek: Arc<dyn overdrive_core::ca::kek::Kek>) -> Self` — body is the old `Default::default()` body with `kek` taken from the argument. `#[must_use]`. |
| `Debug` impl | Extend the manual `Debug` (`lib.rs:689-713`) with `.field("kek", &"<dyn Kek>")` (it is `Arc<dyn Kek>`, not `Debug`). |
| Production composition | At the CLI `serve` boundary (`overdrive-cli::commands::serve` → the site that builds `ServerConfig` for `run_server`): `kek: Arc::new(overdrive_host::ca::SystemdCredsKeyring::new())`. |
| `run_server` consumption | At `lib.rs:1601` (inside `run_server_with_obs_and_driver`): delete `let kek = overdrive_host::ca::SystemdCredsKeyring::new();`; pass `config.kek.as_ref()` to both `boot_ca` and `bootstrap_node_intermediate` (both already take `&dyn Kek`). `boot_ca` / `bootstrap_node_intermediate` / `RootKeyAeadCodec` / `root_kek_id` are UNCHANGED — only the source of the `&dyn Kek` changes. |

### Crafter obligations (C-1 … C-4)

These bind the follow-up implementation dispatch. Cite them by id.

- **C-1 — production wiring change in `run_server`.** Replace the inline
  `let kek = overdrive_host::ca::SystemdCredsKeyring::new();` (`lib.rs:1601`)
  with consumption of `config.kek` (`config.kek.as_ref()` into both `boot_ca`
  and `bootstrap_node_intermediate`). Do NOT change `boot_ca` /
  `bootstrap_node_intermediate` / `RootKeyAeadCodec` / `root_kek_id` — those are
  REUSE-AS-IS. Compose `SystemdCredsKeyring::new()` once, at the CLI `serve`
  boundary, into `ServerConfig.kek`.

- **C-2 — `ServerConfig` seam.** Add `pub kek: Arc<dyn Kek>`; remove
  `impl Default for ServerConfig`; add `ServerConfig::new(kek)` carrying every
  former-`Default` field value; extend the manual `Debug` with the elided `kek`
  field. Do NOT introduce a defaulted or `Option`-override `kek` — the mandatory
  shape is the decision (see § "Seam decision" above); reproducing the
  defaulted/optional hazard is a design divergence, not an implementation choice.

- **C-3 — test-fixture obligation (hermetic KEK, EVERY caller).** EVERY
  `run_server` / `run_server_with_obs_and_driver` caller across
  `crates/overdrive-control-plane/tests/integration/` and
  `crates/overdrive-control-plane/tests/acceptance/` MUST inject a **hermetic**
  test KEK: `overdrive_sim::adapters::SimKek::for_boot()` as the
  `Arc<dyn Kek>` — a pure in-process `Kek` test double
  (`crates/overdrive-sim/src/adapters/kek.rs`) whose `for_boot()` preloads the
  canonical `overdrive-ca-root` KEK from a `BTreeMap`. It uses no kernel keyring,
  no `$CREDENTIALS_DIRECTORY`, and no FFI, so the fixture owns its KEK material
  end-to-end and the suite passes on a cold CI VM. Do NOT rely on process-global
  env (`$CREDENTIALS_DIRECTORY` / `OVERDRIVE_CA_KEK`) or the leaked kernel
  keyring — `SimKek` is keyring-independent by construction, which eliminates the
  leaked-kernel-keyring masking that hid the original regression and stops the
  fixtures contributing to kernel-keyring key accumulation. **Placement
  rationale:** a `Kek` fixture is a *pure in-process test double*, so per
  `.claude/rules/development.md` § "Shared real-infra test fixtures" it belongs
  with the other `Sim*` adapters in `overdrive-sim` (the sim/host split), NOT in
  `overdrive-testing` (which is for *real-OS* fixtures) and NOT a crate-local
  `tests/integration/helpers/` copy. Both consuming crates
  (`overdrive-control-plane`, `overdrive-cli`) already dev-dep `overdrive-sim`,
  so injecting `SimKek::for_boot()` is zero new wiring — no shared-helper module,
  no credential staging, no `TempDir` lifetime to manage.

- **C-4 — corrected gate (RUN, not `--no-run`).** The step's quality gate MUST
  actually execute the `run_server` fixture suite under Lima:
  `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane
  --features integration-tests` (the acceptance suite likewise:
  `… --test acceptance --features integration-tests`). A `--no-run` compile-only
  gate is INSUFFICIENT and is explicitly what let this regression land — a
  `--no-run` build never calls `boot_ca`, so a cold-environment KEK refusal is
  invisible to it. The macOS `--no-run` compile-check (per
  `.claude/rules/testing.md`) remains necessary-but-not-sufficient; the Lima RUN
  is the load-bearing gate that proves the ~26 fixtures actually boot.

### Out of scope for this amendment

- `boot_ca` / `bootstrap_node_intermediate` / `RootKeyAeadCodec` / `root_kek_id`
  — UNCHANGED (REUSE-AS-IS). The ONLY new public surface is `ServerConfig.kek` +
  `ServerConfig::new`.
- The two-CA discipline is intact: this amendment touches ONLY the
  workload-identity CA's KEK source (`lib.rs:1595` path). The operator /
  control-plane HTTPS CA (`mint_ephemeral_ca`, `lib.rs:1237`) is ephemeral by
  design, unrelated, and untouched.
- The `serve_persistent_ca.rs` scaffolds (S-OC-06/07/08a-d/09) STAY `#[ignore]`
  — still owned by the later runtime slice. This amendment does NOT un-ignore
  them; it only makes the *pre-existing* ~26 `run_server` fixtures boot again.

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

**Scope:** `lib.rs:1580-1617` composition + `ServerConfig.kek` injection seam
(§ C1-AMEND) + `ControlPlaneError::CaBoot` + the ~26-fixture test-KEK
injection + the **corrected RUN gate**.

- Add the mandatory `ServerConfig.kek: Arc<dyn Kek>` seam + `ServerConfig::new(kek)`
  constructor; remove `ServerConfig: Default` (§ C1-AMEND).
- Consume `config.kek` in the boot composition (do NOT construct
  `SystemdCredsKeyring::new()` inline); `RootKeyAeadCodec::new()` +
  `root_kek_id()`; coerce `store` → `Arc<dyn IntentStore>`.
- Replace ephemeral `RcgenCa` block: `boot_ca(ca.as_ref(), config.kek.as_ref(), …).await?`
  then `bootstrap_node_intermediate(…, config.kek.as_ref(), …).await?`; build
  bundle from adopted CA.
- Compose `SystemdCredsKeyring::new()` at the CLI `serve` boundary
  (`overdrive-cli::commands::serve`) and pass it as `ServerConfig.kek`.
- Inject a **hermetic** test KEK (`overdrive_sim::adapters::SimKek::for_boot()`,
  see crafter obligation C-3) into EVERY `run_server` /
  `run_server_with_obs_and_driver` fixture across `tests/integration/` +
  `tests/acceptance/`.
- Add `ControlPlaneError::CaBoot(#[from] CaBootError)` + exhaustive `to_response`
  arm (boot-path → 500, exhaustiveness-only).
- **Gate (CORRECTED):** the step MUST actually **RUN** the `run_server` fixture
  suite under Lima — `cargo xtask lima run -- cargo nextest run -p
  overdrive-control-plane --features integration-tests` — not merely `--no-run`
  compile it. The original `--no-run`-only gate is what let the cold-env
  `KekUnavailable` regression land (a `--no-run` build never executes `boot_ca`,
  so the cold-boot refusal is invisible to it). See crafter obligation C-4.
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
  production binding. **The `Kek` production binding (`SystemdCredsKeyring::new()`)
  is composed at the CLI `serve` binary boundary and injected through
  `ServerConfig.kek` — NOT constructed inline inside `run_server` (§ C1-AMEND).**
  Inline construction of a port-trait production binding inside `run_server` is
  the regression this amendment closes: it is the `.claude/rules/development.md`
  § "Port-trait dependencies" violation ("never default the field to a production
  binding … tests that forgot to override" — here the *inline construction*
  forces the production binding on every fixture). `RootKeyAeadCodec` / `RcgenCa`
  remain composed at `run_server` (they have no cold-environment failure mode and
  no test-double the suite needs to inject).
- The `Ca` / `Kek` / `IntentStore` / `ObservationStore` port traits are the
  boundary; `boot_ca` takes `&dyn Ca` / `&dyn Kek` / `&Arc<dyn IntentStore>`. The
  `Kek` `&dyn` argument is sourced from the injected `ServerConfig.kek`, so the
  trait boundary is honored by injection, not by inline binding.

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

---

## Wave: DISTILL

**Density:** lean (Tier-1 `[REF]` only) · **Designer:** Quinn · **Acceptance
SSOT:** `docs/feature/built-in-ca-operator-composition/distill/test-scenarios.md`
(GIVEN/WHEN/THEN are SPECIFICATION ONLY — no `.feature` files; the Rust scaffolds
under `crates/*/tests/{acceptance,integration}` are the executable artifacts).
**RED classification:** `docs/feature/built-in-ca-operator-composition/distill/red-classification.md`.

### [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DESIGN#D-OC-1 | #40 near-expiry reissue is a reconciler `Action::IssueSvid` (`"rotate-svid"`), NOT a workflow | n/a | S-OC-01/05/10 pin the rotate-as-action emit + executor reuse; no StartWorkflow anywhere |
| DESIGN#D-OC-2 | Near-expiry branch emits unconditionally (gate retired) | n/a | S-OC-01 asserts the unconditional emit; the existing gated-seam GREEN test is deleted (single-cut) |
| DESIGN#D-OC-3 | Threshold = ½ × `WORKLOAD_SVID_TTL` (1800s), derived | n/a | S-OC-04 pins the threshold TRACKS TTL via the emitted action (no const inspection); S-OC-03 pins the literal `<=` boundary |
| DESIGN#D-OC-4 | Wire `boot_ca` + `bootstrap_node_intermediate` into `run_server` | n/a | S-OC-06/07 prove persistent boot + adopt-on-restart through the wired `overdrive serve` binary |
| DESIGN#D-OC-5 | `ControlPlaneError::CaBoot` — cause-distinct refuse-to-start | n/a | S-OC-08a/b/c pin one refusal cause each (wrong-KEK / tampered / absent); S-OC-08d pins pairwise-distinct stderr; S-OC-09 pins no silent re-mint |
| DESIGN#D-OC-6 | Restart = re-mint (confirm only) | n/a | S-OC-05/07 keep the restart-recovery branch distinct from rotate |
| DESIGN#D-OC-7 | Additive `AllocStatusResponse.issued_certificates`, latest-by-`issued_at` | n/a | S-OC-11/12 pin the render + the no-cert-bytes / latest-by-`issued_at` projection |
| DESIGN#D-OC-8 | Un-skip the `near_expiry` mutation boundary | n/a | S-OC-03 is the live mutation kill-test for the `<=` boundary |
| review-design#Medium-1 | E03 runner MUST enforce all 3 sub-claims before `satisfied` | n/a | S-OC-13/14/15 specify all three; S-OC-15 is the mandatory pathLen=0 negative anchor |

### [REF] Scenario list (18) + tags

| ID | Title | Tags | Slice | Tier |
|---|---|---|---|---|
| S-OC-01 | Near-expiry held SVID emits one rotate `IssueSvid` | `@dst @property @driving_port @slice-1` | ① | L1 |
| S-OC-02 | Not-near-expiry held SVID emits no `IssueSvid` | `@dst @property @error @driving_port @slice-1` | ① | L1 |
| S-OC-03 | Near-expiry `<=` boundary inclusive at half-TTL (kill-test) | `@dst @error @driving_port @slice-1` | ① | L1 |
| S-OC-04 | Rotation threshold TRACKS ½ × `WORKLOAD_SVID_TTL` via emitted action | `@dst @driving_port @slice-1` | ① | L1 |
| S-OC-05 | Rotate distinct from restart-recovery re-issue | `@dst @property @error @driving_port @slice-1` | ① | L1 |
| S-OC-06 | `serve` first boot generates + seals + persists root | `@integration @real-io @adapter-integration @driving_port @slice-2 @edd:D01` | ② | L3 |
| S-OC-07 | `serve` restart adopts SAME root (no re-mint) | `@integration @real-io @adapter-integration @driving_port @slice-2 @edd:D01` | ② | L3 |
| S-OC-08a | `serve` refuses to start on the WRONG KEK | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 |
| S-OC-08b | `serve` refuses to start on a TAMPERED envelope | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 |
| S-OC-08c | `serve` refuses to start when the KEK is ABSENT | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 |
| S-OC-08d | The three refusal causes render pairwise-distinct stderr | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 |
| S-OC-09 | Refuse-to-start leaves root unchanged (no re-mint) | `@integration @real-io @error @driving_port @slice-2 @edd:O04` | ② | L3 |
| S-OC-10 | Rotate-correlation `IssueSvid` reuses the executor | `@integration @real-io @adapter-integration @driving_port @slice-1` | ① | L3 |
| S-OC-11 | `alloc status` surfaces current issued-cert summary | `@integration @real-io @adapter-integration @driving_port @slice-3 @edd:O05` | ③ | L3 |
| S-OC-12 | Summary omits cert bytes/key; latest-by-`issued_at` | `@integration @real-io @error @driving_port @slice-3 @edd:O05` | ③ | L3 |
| S-OC-13 | Exported chain verifies (`openssl verify`) | `@integration @real-io @adapter-integration @driving_port @slice-3 @edd:E03` | ③ | L3 |
| S-OC-14 | Exported leaf profile (one URI SAN / CA:FALSE / crit digSig) | `@integration @real-io @adapter-integration @driving_port @slice-3 @edd:E03` | ③ | L3 |
| S-OC-15 | pathLen=0 negative anchor FAILS `openssl verify` | `@integration @real-io @error @driving_port @slice-3 @edd:E03` | ③ | L3 |

**Error/edge ratio: 10/18 = 56%** (S-OC-02/03/05/08a/08b/08c/08d/09/12/15) — ≥ 40% met.

### [REF] Scaffolds created / modified

| File | Type | Scenarios |
|---|---|---|
| `crates/overdrive-core/tests/acceptance/svid_lifecycle_rotation.rs` | NEW (`#[should_panic(expected = "RED scaffold")]`) | S-OC-01..05 |
| `crates/overdrive-core/tests/acceptance.rs` | MODIFY (wire `mod svid_lifecycle_rotation;`) | — |
| `crates/overdrive-control-plane/tests/integration/built_in_ca_operator_composition/serve_persistent_ca.rs` | NEW (`#[ignore]`, Lima-gated) | S-OC-06/07/08a/08b/08c/08d/09 |
| `crates/overdrive-control-plane/tests/integration/built_in_ca_operator_composition/rotate_issue_svid_dispatch.rs` | NEW (`#[ignore]`, Lima-gated) | S-OC-10 |
| `crates/overdrive-control-plane/tests/integration/built_in_ca_operator_composition/alloc_status_issued_certificates.rs` | NEW (`#[ignore]`, Lima-gated) | S-OC-11/12 |
| `crates/overdrive-control-plane/tests/integration.rs` | MODIFY (wire `mod built_in_ca_operator_composition`) | — |
| `crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs` | MODIFY (DELIVER-obligation note for `OD_E03_CA_DIR` export hook; existing tests stay GREEN) | S-OC-13/14/15 |

E03 (S-OC-13/14/15) has NO new scaffold: it reuses the EXISTING GREEN
`rcgen_ca_chain_verify.rs` tests; Slice ③ adds an env-gated PEM export (a test
fixture change, NOT a behavioural one — the tests stay GREEN). DISTILL does NOT
edit `verification/expectations/E03-…/runner.sh` (DELIVER owns the 3-check
extension + export-hook wiring).

### [REF] Driving Adapter coverage

| Driving adapter | Real-protocol scenarios | Mechanism |
|---|---|---|
| `overdrive serve` (CLI) | S-OC-06/07/08a/08b/08c/08d/09 | Real subprocess in Lima (boot / restart / refuse-to-start ×3 + pairwise-distinct stderr) |
| `overdrive alloc status --job <id>` (CLI) | S-OC-11/12 | Real subprocess in Lima (issued-cert summary render) |
| `SvidLifecycle::reconcile` (domain port) | S-OC-01..05 | Direct pure call (Tier-1 DST) |
| `IssueSvid` action-shim executor | S-OC-10 | Direct dispatch, real CA + ObservationStore |
| `rcgen_ca_chain_verify` test + `openssl` | S-OC-13/14/15 | Real mint → PEM export → `openssl verify` subprocess |

### [REF] Adapter coverage (driven, ≥1 real-I/O Tier-3)

| Driven adapter | Real-I/O scenarios |
|---|---|
| `Ca` / `RcgenCa` | S-OC-06/07/10/13/14/15 |
| `Kek` / `SystemdCredsKeyring` | S-OC-06/07/08a/08b/08c/09 |
| `IntentStore` / `LocalIntentStore` (redb) | S-OC-06/07/09 |
| `ObservationStore` / `LocalObservationStore` | S-OC-10/11/12 |

### [REF] EDD mapping (graduation)

| Expectation | Slice | Graduating scenarios | Capture surface |
|---|---|---|---|
| D01 (root key never plaintext at rest) | ② | S-OC-06/07 | on-disk IntentStore byte-scan (built binary) |
| O04 (refuse-to-start, actionable) | ② | S-OC-08a/08b/08c/08d/09 | `overdrive serve` stderr (3 cause-distinct, one scenario each + pairwise-distinct contract) + no re-mint |
| O05 (issued-certificates audit row) | ③ | S-OC-11/12 | `overdrive alloc status` render (no cert bytes) |
| E03 (full chain verifies) | ③ | S-OC-13/14/15 | exported-PEM `openssl verify` (ALL 3 sub-claims) |

Slice ① scenarios (S-OC-01..05, S-OC-10) do NOT graduate to EDD — pure
in-process reconciler logic + action-shim dispatch, no new operator surface.
**E03 is satisfiable ONLY when the runner enforces sub-claims 1–3**; the
different-fox Haiku reviewer per expectation MUST refute E03 evidence missing the
pathLen=0 negative anchor (S-OC-15).

### [REF] Test placement + pre-requisites

- **Placement**: Slice ① pure → `overdrive-core/tests/acceptance/`
  (default lane, no Lima); Slice ①/②/③ real-I/O → `overdrive-control-plane/tests/integration/built_in_ca_operator_composition/`
  (gated `integration-tests`, Lima); E03 → existing `overdrive-host/tests/integration/rcgen_ca_chain_verify.rs`.
- **Pre-requisites** (DESIGN driving ports + environment): `overdrive serve` and
  `overdrive alloc status` CLI verbs (existing); real `RcgenCa` / `SystemdCredsKeyring`
  / `LocalIntentStore` (redb) / `LocalObservationStore`; Lima VM with cgroup v2 +
  systemd-creds/keyring for the boot subprocess; `openssl` on PATH (E03). DEVOPS
  delta absent → single-node default + existing integration-test/Lima policy
  (warning, not blocker). `cargo xtask bpf-build` is a compile prereq for the
  control-plane integration binary.
- **Compile-check (this run, via Lima)**: `overdrive-core --test acceptance --no-run`
  and `overdrive-control-plane --test integration --features integration-tests --no-run`
  both GREEN; `svid_lifecycle_rotation` 5 scaffolds run as 5 passed (RED, not
  BROKEN). Integration scaffolds `#[ignore]` until per-slice wiring lands.

### [REF] Outcome-registration candidates (OUT-N)

The orchestrator registers these (DISTILL has no Bash CLI access for
`nwave-ai outcomes register`). Two genuinely-new typed contract surfaces:

| id | kind | input-shape | output-shape | keywords |
|---|---|---|---|---|
| `OUT-OC-ISSUED-CERTS-READ` | operation | `overdrive alloc status --job <WorkloadId>` over `issued_certificate_rows()` | `AllocStatusResponse.issued_certificates: Vec<IssuedCertSummary { serial, spiffe_id, issuer_serial, not_after }>` — latest-by-`issued_at` per running alloc, NO cert bytes / NO key | issued-certificates, alloc-status, operator-read, audit, svid-summary |
| `OUT-OC-CA-BOOT-REFUSE` | invariant | `run_server` boot with persisted root + (resolvable \| wrong \| absent) KEK / (intact \| tampered) envelope | refuse-to-start with cause-distinct `ControlPlaneError::CaBoot(CaBootError)` (`WrongKek` \| `TamperedEnvelope` \| `KekUnavailable`) + `health.startup.refused`, NO silent re-mint (root unchanged) | ca-boot, refuse-to-start, earned-trust, no-remint, kek, envelope |

The rotate-as-action behaviour (S-OC-01..05/10) does NOT register a new outcome:
it reuses the existing `OUT-WIM-SVID-LIFECYCLE` contract (`Action::IssueSvid`
unchanged) — a behavioural extension of an already-registered surface, not a new
typed contract.
