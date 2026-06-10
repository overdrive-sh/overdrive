# DESIGN Wave Decisions — `built-in-ca-operator-composition`

**Mode:** propose (guide-mode rulings settled at dispatch) · **Paradigm:** OOP
Rust · **Scope:** Application · **Architect:** Morgan

## Key Decisions

| ID | Decision | Status |
|---|---|---|
| D-OC-1 | #40 near-expiry reissue = reconciler **action** (`Action::IssueSvid` rotate-correlation), NOT a workflow | Settled |
| D-OC-2 | Retire the `ROTATION_ENABLED` gate + `cert_rotation` workflow name; near-expiry emits `IssueSvid` unconditionally | Settled |
| D-OC-3 | Near-expiry threshold = ½ × `WORKLOAD_SVID_TTL` = **1800s** (verified: TTL is 3600s) | Settled |
| D-OC-4 | Wire `ca_boot::boot_ca` + `bootstrap_node_intermediate` into `run_server` (`lib.rs:1595`), replacing the ephemeral `RcgenCa` block | Settled |
| D-OC-5 | `ControlPlaneError::CaBoot(#[from] CaBootError)` — dedicated variant, never flatten to `Internal` | Settled |
| D-OC-6 | Restart = re-mint; #35's `ever_issued → IssueSvid` branch correct as-is (confirm only) | Confirmed |
| D-OC-7 | Additive `AllocStatusResponse.issued_certificates: Vec<IssuedCertSummary>`, latest-by-`issued_at` per running alloc | Settled |
| D-OC-8 | Un-skip the `near_expiry` mutation boundary (live target) | Settled |
| D-OC-9 | `Kek` provider injected through a **mandatory** `ServerConfig.kek: Arc<dyn Kek>` field (remove `ServerConfig: Default`, add `ServerConfig::new(kek)`); production composes `SystemdCredsKeyring::new()` at the CLI `serve` boundary, tests inject a hermetic fixture KEK — replaces C1's inline `SystemdCredsKeyring::new()` in `run_server` | Settled (DELIVER-review amendment, 2026-06-10) |

## Architecture Summary

A composition + lifecycle-completion feature over the shipped `built-in-ca`
(ADR-0063) and `workload-identity-manager` (ADR-0067) subsystems. Three moves:

1. **Rotation as action.** `SvidLifecycle::reconcile`'s near-expiry branch flips
   from a gated `StartWorkflow(cert_rotation)` to an unconditional
   `Action::IssueSvid` (rotate correlation), reusing the existing variant. The
   internal SVID reissue is a single mint+swap — not a ≥2-external-step
   workflow.
2. **Persistent CA at boot.** `run_server` calls the already-implemented,
   already-probing `boot_ca` + `bootstrap_node_intermediate` (KEK probe → envelope
   decrypt-probe → adopt-or-refuse), replacing the ephemeral `RcgenCa` composition.
   Refuse-to-start surfaces a typed `CaBoot` error.
3. **Operator-visible current SVID.** The `alloc status` read aggregates the
   append-only `issued_certificates` audit and projects the current cert (latest
   row per running alloc by `issued_at`) into an additive response field.

Style: Hexagonal ports-and-adapters (established). No new subsystem, no new
dependency, no new public API surface beyond one additive wire struct.

## Reuse Analysis

See feature-delta.md § Reuse Analysis (HARD GATE). Summary: **11 REUSE AS-IS, 2
EXTEND (additive), 3 DELETE (single-cut), 0 CREATE-NEW** beyond the
`IssuedCertSummary` wire struct. `boot_ca`, the `IssueSvid` executor, the `Kek` /
`Ca` adapters, and the audit row are all reused as-is — only newly *called* or
*projected*.

## Tech Stack

No new dependencies. Reuses `ring`/`aws-lc-rs` (AES-GCM envelope), `rcgen`
(`RcgenCa`), `redb` (`IntentStore`), Corrosion/CR-SQLite (`ObservationStore`),
systemd-creds/kernel keyring (`SystemdCredsKeyring`) — all already in graph.

## Constraints

- **Single-node (Phase 2.6).** One node → one intermediate (multi-node = #36).
- **Leaf keys never at rest** (ADR-0063 D9) → restart = re-mint, driven by the
  audit-row `ever_issued` signal (ADR-0067 rev 5 D10).
- **No new public API surface** (CLAUDE.md "Implement to the design"): the rotate
  path reuses `Action::IssueSvid` UNCHANGED — no new field, flag, or variant.
- **Earned Trust:** the boot path must `probe-then-use`; the probes already exist
  in `boot_ca`, this feature wires them so they run at production boot.
- **`out-of-scope`:** `mint_ephemeral_ca()` (operator/CP-HTTPS CA, lib.rs:1237,
  D-CA-5/#81); whitepaper §18; ADR-0064/0065/0066.

## Upstream Changes (ADR/doc corrections — all 5)

| Target | Change |
|---|---|
| ADR-0067 (rev 6) | A5 reframe (rotation = permanent reconciler *action*, not throwaway sync-rotate); D8 + #40-boundary rewrite (emit `IssueSvid`, drop the wait-for-DNS-propagation workflow fiction); D1/D8 restart-re-mint re-validation note |
| ADR-0063 (dated amendment) | #215 wires `boot_ca`/`bootstrap_node_intermediate` into `run_server` (closes D-CA-4 "CA not wired into serve"); records `ControlPlaneError::CaBoot` + O04 cause-distinctness; D01/O04 pending→wired |
| `.claude/rules/workflows.md` § "Codebase precedent" | Correct "canonical first workflow = certificate rotation (#40) … wait for DNS propagation"; no first-party production workflow ships yet; candidate first = TBD (revocation-coupled rotation, Phase 5); internal SVID near-expiry reissue is a reconciler action |
| `docs/product/architecture/brief.md` | New `### Built-in CA operator composition` subsection under `## Application Architecture` |
| `.cargo/mutants.toml` | (DELIVER) remove the `"near_expiry"` `exclude_re` entry — the boundary is a live mutation target |

## DELIVER slices

① Rotation (core action flip + gate retire + un-skip mutation) · ② Boot-side
#215 (`boot_ca` wiring + `ControlPlaneError::CaBoot` + O04) → captures D01, O04 ·
③ Consumer-side #215, **two distinct surfaces with distinct proofs**:

- **`issued_certificates` field + CLI render → captures O05 ONLY.** Operator-legible
  metadata (`serial / spiffe_id / issuer_serial / not_after`, NO cert bytes, NO key).
  This render does NOT prove the chain verifies and MUST NOT be treated as
  satisfying E03.
- **E03 (full chain verifies) → captured SEPARATELY by the test-only `OD_E03_CA_DIR`
  PEM export** from `crates/overdrive-host/tests/integration/rcgen_ca_chain_verify.rs`
  plus `verification/expectations/E03-ca-full-chain-verifies/runner.sh`. **The
  current runner enforces ONLY sub-claims 1–2 then exits; Slice ③ MUST extend
  `runner.sh` to enforce ALL THREE E03 sub-claims before any `satisfied`:**
  (1) chain verifies — `openssl verify -CAfile root.pem -untrusted intermediate.pem
  svid.pem` → OK; (2) leaf profile — exactly one `spiffe://` URI SAN, `CA:FALSE`,
  critical `digitalSignature`; (3) the pathLen=0 negative anchor (S-03-05) — a chain
  where the pathLen=0 intermediate signs a *further CA* MUST FAIL `openssl verify`
  (pathLen *enforced*, not merely *set*). Sub-claim 3's source is named: under the
  same `OD_E03_CA_DIR` env-gate, export the further-CA chain from the existing test
  `rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced` and assert
  `openssl verify` FAILS (or capture that test's own failing-verification evidence).
  No production API surface and no operator verb mints/exports a chain (D-CA-4).
  See `feature-delta.md` § E03/O05 split for the full runner-wiring contract this
  mirrors.

**E03 is NOT satisfiable until the runner enforces sub-claims 1–3.** EDD per slice
via Lima `run-expectation.sh`, different-fox Haiku reviewer per expectation before
any `satisfied`. An agent must NOT mark E03 satisfied off the summary render (the PEM
`openssl verify` capture is the only E03 proof) AND must NOT mark E03 satisfied off
the present 2-check runner — the different-fox reviewer MUST reject E03 evidence that
omits the sub-claim-3 negative anchor.

## Amendment — `Kek` injection seam (D-OC-9, DELIVER-review-driven, 2026-06-10)

**Trigger.** DELIVER review of the boot-wiring step (Slice ②) surfaced a
regression in the originally-pinned **C1**. C1 said "construct
`SystemdCredsKeyring::new()` … in `run_server`" — i.e. hardcode the production
`Kek` binding inline at the composition root with no injection seam.
Consequence: every test fixture that boots through `run_server` /
`run_server_with_obs_and_driver` (~26 callers across
`crates/overdrive-control-plane/tests/integration/` + `tests/acceptance/`) now
hits `boot_ca` → `SystemdCredsKeyring::new().resolve("overdrive-ca-root")`,
which in a **cold environment** (no `$CREDENTIALS_DIRECTORY`, no
`OVERDRIVE_CA_KEK` dev-opt-in, empty kernel keyring) returns
`KekError::NotFound` → `CaBootError::KekUnavailable` → boot refuses → the
fixture panics at `.expect("run_server")`. Masked locally by a leaked
persistent kernel-keyring key (`overdrive:ca:kek:overdrive-ca-root`, unknown
provenance); confirmed by invalidating that key:

```
panicked at server_lifecycle.rs:105:65:
run_server: CaBoot(KekUnavailable { kek_id: KekId("overdrive-ca-root"),
                                    source: NotFound { kek_id: KekId("overdrive-ca-root") } })
```

On a fresh CI VM all ~26 callers fail identically.

**Root cause.** Exactly the anti-pattern in `.claude/rules/development.md`
§ "Port-trait dependencies — `overdrive-host` is production, `overdrive-sim` is
tests": *"Never default the field to a production binding inside the
constructor — that silently inherits … behaviour into tests that forgot to
override, which is the exact failure mode the trait surface exists to
prevent."* `Kek` (`overdrive_core::ca::kek::Kek`) is a port trait, and the
inline `SystemdCredsKeyring::new()` forced its production binding on every boot
site — including every test fixture. `boot_ca` / `bootstrap_node_intermediate`
already take `&dyn Kek`, so the trait surface was right there; C1 simply failed
to route the seam through it.

**Decision (user-approved "Option A").** Thread the `Kek` provider through
`ServerConfig` as a **mandatory** field:

- New field `pub kek: Arc<dyn overdrive_core::ca::kek::Kek>` on `ServerConfig`.
- **Remove `impl Default for ServerConfig`** (a mandatory `Arc<dyn Kek>` cannot
  be defaulted to a benign value — any default would be a second hidden
  production-or-fake binding, the same hazard). Add
  `ServerConfig::new(kek: Arc<dyn Kek>) -> Self` carrying every former-`Default`
  field value; fixtures swap `..Default::default()` → `..ServerConfig::new(test_kek())`.
- `run_server` consumes `config.kek.as_ref()` into `boot_ca` /
  `bootstrap_node_intermediate` (both unchanged). Production composes
  `SystemdCredsKeyring::new()` at the CLI `serve` boundary; tests inject a
  hermetic `overdrive_sim::adapters::SimKek::for_boot()` — a pure in-process
  `Kek` test double (`crates/overdrive-sim/src/adapters/kek.rs`) that preloads
  the canonical `overdrive-ca-root` KEK from a `BTreeMap`, with no kernel
  keyring, no `$CREDENTIALS_DIRECTORY`, and no FFI.

**Why mandatory — not defaulted, not optional-override.** `ServerConfig`
carries both idioms (`clock` is defaulted; `dataplane_override` is an
`Option`), and both reproduce the regression for THIS trait:

- A **defaulted** `kek` mirrors `clock`, but `clock`'s forgotten default
  (`SystemClock`) is *benign* (a test silently uses wall-clock — a smell, but
  it boots) whereas `kek`'s forgotten default (`SystemdCredsKeyring::new()`) is
  *malign* — it refuses to boot cold, and the compiler does NOT catch the
  omission. This is the exact "tests can forget" failure just observed.
- An **`Option<Arc<dyn Kek>>` override** mirrors `dataplane_override` and is
  minimal-churn, but development.md explicitly calls optional/builder overrides
  an anti-pattern *for port traits* — `None → SystemdCredsKeyring::new()` means
  a `..Default::default()` fixture that forgets the override silently gets the
  cold-failing production KEK. Same hazard, spelled with `Option`.
- A **mandatory** `kek` is development.md's stated preference ("fails to compile
  rather than silently running on the production binding") and is the ONLY shape
  where a forgotten KEK is a **compile error**, not a cold-boot refusal.

**Churn is identical across all three shapes** (every fixture already uses an
explicit `ServerConfig { … , ..Default::default() }` literal, so each forgetting
fixture adds exactly one line regardless of shape), so churn is not a
tie-breaker — and given equal churn, the shape that turns the omission into a
compile error wins. The accepted cost is removing `ServerConfig: Default`; the
`ServerConfig::new(kek)` constructor preserves rest-pattern ergonomics for every
other field.

**Crafter obligations (C-1 … C-4)** are pinned in
`feature-delta.md` § C1-AMEND: (C-1) production wiring change in `run_server`
(consume `config.kek`, do not construct inline); (C-2) the `ServerConfig` seam
(mandatory field, remove `Default`, add `new(kek)`, extend `Debug`); (C-3) the
hermetic test-KEK obligation for EVERY `run_server` caller —
inject `overdrive_sim::adapters::SimKek::for_boot()` as the `Arc<dyn Kek>` (a
pure in-process `Kek` double in `overdrive-sim`, keyring-independent by
construction; per `.claude/rules/development.md` § "Shared real-infra test
fixtures" a pure in-process double belongs with the `Sim*` adapters, NOT in
`overdrive-testing` and NOT a crate-local helper — both consuming crates already
dev-dep `overdrive-sim`, so zero new wiring); (C-4) the corrected gate —
actually **RUN** the fixture suite under Lima (`cargo xtask lima run -- cargo
nextest run -p overdrive-control-plane --features integration-tests`), since the
original `--no-run`-only gate never executes `boot_ca` and so cannot see a
cold-boot refusal.

**Scope discipline.** This amendment touches ONLY the `Kek` source. `boot_ca` /
`bootstrap_node_intermediate` / `RootKeyAeadCodec` / `root_kek_id` are
REUSE-AS-IS; the only new public surface is `ServerConfig.kek` +
`ServerConfig::new`. The two-CA discipline is intact — the operator /
control-plane HTTPS CA (`mint_ephemeral_ca`, `lib.rs:1237`) is unrelated and
untouched. The `serve_persistent_ca.rs` scaffolds stay `#[ignore]` (later
runtime slice).

**Test-helper reconciliation (2026-06-10).** The hermetic test-KEK mechanism in
C-3 was reconciled from the originally-pinned `SystemdCredsKeyring::with_credentials_dir(tempdir)`
+ staged-credential helper (placed crate-local) to
`overdrive_sim::adapters::SimKek::for_boot()` — a pure in-process `Kek` double
in `overdrive-sim` (`crates/overdrive-sim/src/adapters/kek.rs`) — as the
cleaner, keyring-independent choice, sanctioned during DELIVER. Rationale: a
`Kek` fixture is a pure in-process test double, so per
`.claude/rules/development.md` § "Shared real-infra test fixtures" it belongs
with the `Sim*` adapters (the sim/host split), NOT in `overdrive-testing`
(real-OS fixtures only); `SimKek` is keyring-independent by construction, which
eliminates the leaked-kernel-keyring masking that hid the original regression and
stops the fixtures accumulating kernel-keyring keys; and both consuming crates
already dev-dep `overdrive-sim`, so injection is zero new wiring. **The
production seam (D-OC-9 above) is unchanged** — mandatory `kek`, `Default`
removed, `ServerConfig::new(kek)`, production composes
`SystemdCredsKeyring::new()` at the CLI `serve` boundary, `run_server` consumes
`config.kek` — only the test-double the suite injects was reconciled.
