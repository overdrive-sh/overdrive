# ADR-0010 — Phase 1 TLS bootstrap: ephemeral in-process CA, embedded trust triple in `~/.overdrive/config`

## Status

Accepted. 2026-04-23. **§R2 superseded 2026-04-24 by ADR-0019**
(on-disk format YAML → TOML; every other aspect of R2 — field names,
context model, `current-context` pointer, base64 PEM embedding,
`OVERDRIVE_CONFIG` env override, kubeconfig-shape ergonomics,
Phase 5 forward-compat — preserved bit-for-bit). **§R1 and §R4
amended 2026-04-26** (Phase 1 has exactly one cert-minting site, and
it is `overdrive serve`; the `cluster init` arm of the original §R1
disjunction and the `cluster init --force` recovery clause in §R4 are
removed from Phase 1; both return in Phase 5 with the persistent CA
and operator-cert ceremony they require — see Amendment 2026-04-26
below and GH #81). §R3, §R5 remain in force as originally
written; §R5 in particular is the constraint that *required* the
amendment.

## Context

The Phase 1 control-plane REST endpoint binds `https://127.0.0.1:7001`
(ADR-0008). rustls demands a certificate. Phase 5 is where operator
mTLS + SPIFFE operator IDs + Corrosion-gossiped revocation land (user
memory: `project_cli_auth`). Phase 1 has to bridge the gap without
any of that machinery, and without shipping a posture that makes the
Phase 5 migration harder.

The peer re-review of DISCUSS flagged the TLS certificate strategy
gap as medium severity — resolved by `docs/research/security/talos-bootstrap-tls-strategy-comprehensive-research.md`,
which delivered R1–R5 as a self-contained DESIGN-ready recommendation
derived from Talos Linux, kubeadm, Nomad, and FoundationDB primary
sources.

The research finding relevant to this decision: the dominant idiom
across the comparable platforms (Talos, kubeadm, Nomad) is a
self-generated CA baked into the operator's CLI config at
provisioning time, distributed out-of-band, **never using TOFU or
fingerprint pinning**, and **never shipping a `--insecure` escape
hatch outside a narrow maintenance window**.

## Decision

**Phase 1 adopts Talos research recommendations R1–R5 wholesale. The
Phase 1 CA is ephemeral (in-process only), the trust triple is
base64-embedded in `~/.overdrive/config`, and no `--insecure` flag
exists.**

Concretely:

### R1 — Ephemeral in-process CA at `overdrive serve` (amended 2026-04-26)

On every `overdrive serve` start, the binary generates in-memory:

- A self-signed CA (P-256, `rcgen` — already in workspace).
- A server leaf certificate signed by that CA, presented on
  `:7001`.
- A client leaf certificate signed by the same CA, handed to the
  operator through `~/.overdrive/config`.

The CA private key lives in process memory only. No persistence.
Process stop discards the CA. Re-starting `overdrive serve` re-mints
everything.

**Phase 1 has exactly one cert-minting site, and it is `serve`.**
The original §R1 named `cluster init` *or* the server startup path
as triggers; that disjunction was a Phase 5 ceremony shipped early
and is removed (commit `d294fb8`, RCA
`docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`).
`cluster init` returns in Phase 5 with the persistent CA, operator
SPIFFE IDs, Corrosion-gossiped revocation, and the Talos two-file
operatorconfig/machineconfig split that give the verb its meaning
(GH #81). See *Amendment 2026-04-26* below.

### R2 — Base64-embedded trust triple in `~/.overdrive/config`

`~/.overdrive/config` is YAML with the same shape as `~/.talos/config`
and `~/.kube/config`:

```yaml
contexts:
  - name: local
    endpoint: https://127.0.0.1:7001
    ca:  <base64-encoded PEM CA cert>
    crt: <base64-encoded PEM client leaf cert>
    key: <base64-encoded PEM client leaf private key>
current-context: local
```

The CLI reads only from this file (plus an `OVERDRIVE_CONFIG` env
var override — path override, not content override). Environment
variables do not carry cert material.

### R3 — Multi-SAN server leaf cert

The server leaf cert carries SANs:
- `IP:127.0.0.1`
- `IP:::1`
- `DNS:localhost`
- `DNS:<gethostname(3)>`

CN is set to `<hostname>` for older-tooling compatibility but is
not load-bearing — rustls verifies via SAN.

### R4 — No `--insecure` flag (amended 2026-04-26)

No CLI flag bypasses server-cert verification. There is no pre-PKI
window (the CA is minted before `bind()` is called), so the flag
would have nothing to justify its existence. Recovery on lost
client cert in Phase 1 is to **stop and restart `overdrive serve`** —
the next start mints a fresh trust triple and writes it to
`~/.overdrive/config`, which is the only durable artefact (§R5).
The original §R4 recovery clause named
`overdrive cluster init --force`; that verb no longer exists in
Phase 1 (commit `d294fb8`, RCA
`docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`)
and the `--force` reservation moves with it to Phase 5 (GH #81).

### R5 — Defer rotation / revocation / roles / persistence to Phase 5

Phase 1:
- No `revoked_operator_certs` table (Phase 5).
- No `acceptedCAs` multi-CA trust window (Phase 5, CA rotation).
- No `--roles` flag on client-cert mint (Phase 5 RBAC + SPIFFE).
- No cert persistence on disk in the server process (re-init re-mints).

The operator's `~/.overdrive/config` is the only durable artifact.
Losing it is a re-init event, not a recovery event.

### Overdrive-specific divergence from Talos

**One explicit divergence from the research recommendation**: Talos
encodes operator role in the client cert's Organization (O) field.
Overdrive will NOT do this. Per whitepaper §8 (Operator Identity
and CLI Authentication), operator roles are encoded as SPIFFE URI
SANs (`spiffe://overdrive.local/operator/...`) when Phase 5 lands.
In Phase 1 there is no role encoded at all — the client cert
carries an opaque identity field; the CLI is unauthenticated-local
as far as RBAC is concerned. No `operator:admin`/`operator:reader`
distinctions exist yet.

This divergence is documented here so Phase 5 does not retrofit the
wrong identity-in-cert shape from the Talos mirror.

## Considered alternatives

### Alternative A — Use a persisted on-disk CA

**Rejected.** Persistence implies a CA private key file, key
rotation, filesystem permissions discipline, and an on-disk
format. All of this is Phase 5 work. The "ephemeral" choice
trades durability for implementation surface, consistent with the
walking-skeleton framing.

### Alternative B — Plaintext HTTP on `:7001` (no TLS)

**Rejected.** Phase 5 operator-auth requires mTLS. Shipping a
plaintext Phase 1 posture and then migrating to TLS is a breaking
change for every integration test and every operator workflow.
Localhost TLS with a self-signed CA costs ~100 LoC of `rcgen` and
closes the door.

### Alternative C — TOFU / fingerprint pinning

**Rejected.** The research explicitly finds Talos, kubeadm, Nomad,
and FoundationDB all reject TOFU as a trust model. CA-pinned
out-of-band distribution is the universal idiom. Shipping TOFU
in Phase 1 would contradict the structural-security framing of the
platform.

### Alternative D — System-trust CA (ACME for localhost)

**Rejected.** ACME Phase 5+ via `instant-acme` in the gateway
subsystem (whitepaper §11) is the real public-trust path. ACME
for `127.0.0.1` requires DNS-01 or a local ACME server like
`step-ca` — heavy weight for a local dev endpoint.

## Consequences

### Positive

- No `--insecure` escape hatch exists anywhere in the code.
- Phase 5 mTLS, rotation, revocation, and RBAC are additive on top
  of the Phase 1 config shape — no file-format migration.
- Upgrade mechanism: when Phase 5 lands, operators run
  `overdrive cluster upgrade --auth` (or equivalent), which replaces
  the ephemeral in-process CA with a persisted one, rotates the client
  cert, and updates the trust triple in `~/.overdrive/config` — the
  same file format accommodates both. The specific command and
  persistence strategy are Phase 5 DESIGN work; this ADR guarantees
  only the forward-compatible shape.
- `~/.overdrive/config` shape mirrors `~/.talos/config` and
  `~/.kube/config`, lowering cognitive load for operators.
- `rcgen` + `rustls` stay in pure-Rust; design principle 7 honoured.

### Negative

- Losing the operator config requires re-init; no recovery path.
  Acceptable for Phase 1 (walking skeleton), documented explicitly.
- Any change of hostname after init invalidates the SAN match —
  a re-init is required. Rare for a local dev endpoint; documented.

### Quality-attribute impact

- **Security — confidentiality**: TLS 1.3 by default via rustls.
- **Security — authenticity**: server cert verified via embedded
  CA; no TOFU.
- **Usability — operability**: ~/.overdrive/config shape matches
  Talos and kubeconfig.
- **Maintainability — modifiability**: Phase 5 upgrades are
  additive; the current file shape accommodates future
  multi-context, multi-cluster semantics.

### Enforcement

- `rustls::ClientConfig` in the CLI loads the CA from the config
  file's `ca` field. No `DangerousClientConfigBuilder`, no
  `dangerous_accept_any_server_cert`. A grep-gate in CI asserts no
  `dangerous*` rustls API appears in `overdrive-cli` or
  `overdrive-control-plane`.
- A compile-fail test asserts no Phase 1 code path builds a client
  config without CA verification.
- The config file loader rejects any context missing any of
  `ca`/`crt`/`key`.

## References

- `docs/research/security/talos-bootstrap-tls-strategy-comprehensive-research.md`
  (R1–R5, primary sources)
- `docs/whitepaper.md` §8 (Identity and mTLS, Operator Identity)
- User memory: `project_cli_auth`
- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
  (System Constraints: "Auth posture: unauthenticated local endpoint")

## Changelog

- 2026-04-23 — Remediation pass (Atlas peer review, APPROVED-WITH-NOTES):
  added explicit upgrade-mechanism bullet to Consequences → Positive,
  describing how operators move from Phase 1 ephemeral CA to Phase 5
  persistent CA without a file-format migration. Mechanism TBD in
  Phase 5 DESIGN; this ADR guarantees forward-compatibility only.
- 2026-04-24 — §R2 superseded by ADR-0019. On-disk format swapped
  YAML → TOML; all other R2 content (field names, `current-context`,
  `[[contexts]]` semantics, base64 PEM embedding, `OVERDRIVE_CONFIG`
  env override, kubeconfig-shape ergonomics, Phase 5 forward-compat)
  preserved bit-for-bit. Rationale: consistency with every other
  operator-facing config surface in the project (already TOML per
  ADR-0002, ADR-0003, whitepaper §§4, 6, 11, 23); `serde_yaml` was
  archived upstream (design principle 7 better served by `toml`);
  YAML 1.1 footguns (Norway problem, octal coercion, sexagesimal,
  indentation-sensitive misparse) eliminated by construction. R1,
  R3, R4, R5 unchanged. See ADR-0019 for full rationale and
  considered alternatives (including JSON).
- 2026-04-26 — §R1 and §R4 amended in place to encode that Phase 1
  has exactly one cert-minting site, and it is `overdrive serve`.
  See *Amendment 2026-04-26* below for full rationale, before/after
  R-clause text, and cross-references.

## Amendment 2026-04-26 — `cluster init` removed from Phase 1; `serve` is the sole minter

### Date

2026-04-26.

### Rationale (one paragraph, dominant root cause B5 from the RCA)

`overdrive cluster init` and `overdrive serve` both unconditionally
minted a fresh ephemeral CA and wrote a trust triple to
`<config_dir>/.overdrive/config`. With both targeting the production
default `$HOME/.overdrive/`, `serve` ran second and overwrote the
trust triple `cluster init` had just produced — operators were left
with the now-stale CA and handshakes failed. The original §R1
disjunction (`cluster init` *or* server startup as triggers for
minting) was a Phase 5 / Talos-shape ceremony shipped early; §R5
(*"no cert persistence on disk in the server process"*) is precisely
what *prevents* Phase 1 from honouring the operator artefact
`cluster init` produces — every `serve` boot must re-mint, and the
operator's `cluster init`-issued cert is unsignable by the next-boot
server. Backwards-chain validation (RCA §6) falsified every
"make serve consume an existing triple" / "split mint from
endpoint-record" / "Talos two-file split" remediation under Phase 1
constraints; only deletion-in-Phase-1 + Phase-5-reintroduction
passed. §R5 is the constraint that *required* the amendment, not a
defect — it is preserved unchanged. Full backwards-chain analysis
in `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`.

### R-clause changes

**§R1 — before** (original 2026-04-23 text):

> *On first `overdrive cluster init` (or its Phase 1 equivalent
> entry point — the server binary's startup path), the binary
> generates in-memory: A self-signed CA […] A server leaf
> certificate […] A client leaf certificate […] Re-starting
> `cluster init` (or `--force`) re-mints everything.*

**§R1 — after** (this amendment):

> *On every `overdrive serve` start, the binary generates in-memory:
> A self-signed CA […] A server leaf certificate […] A client leaf
> certificate […] Re-starting `overdrive serve` re-mints everything.*
> *Phase 1 has exactly one cert-minting site, and it is `serve`.*

**§R4 — before** (original 2026-04-23 text):

> *Recovery on lost client cert is `overdrive cluster init --force`,
> not a verification-skip.*

**§R4 — after** (this amendment):

> *Recovery on lost client cert in Phase 1 is to stop and restart
> `overdrive serve` — the next start mints a fresh trust triple and
> writes it to `~/.overdrive/config`, which is the only durable
> artefact (§R5).*

§R3 (multi-SAN cert) and §R5 (defer rotation/revocation/roles/
persistence to Phase 5) are unchanged. The Overdrive-specific
divergence from Talos (no role in cert O-field) is unchanged.

### Phase 5 reintroduction

`cluster init` returns in Phase 5 with the invariants that give it
meaning: a persistent CA private key on disk, operator SPIFFE IDs
(`spiffe://overdrive.local/operator/...` per whitepaper §8),
Corrosion-gossiped operator-cert revocation
(`revoked_operator_certs` table, also whitepaper §8), the Talos
two-file `operatorconfig` / `machineconfig` split, the `--force`
non-destructive-modes reservation, and the operator-cert ceremony
(`overdrive op create` / `overdrive op revoke`) the verb actually
implies. Phase 5 reintroduction is tracked in **GH #81**. The
forward-compatible config-file shape preserved by ADR-0019
(kubeconfig-shape `[[contexts]]` array-of-tables) survives the
deletion unchanged; Phase 5 lands on top, not as a migration.

### Cross-references

- RCA: `docs/analysis/root-cause-analysis-cluster-init-cert-overwritten-by-serve.md`
  (multi-causal Toyota 5 Whys; B5 dominant root cause; backwards-chain
  validation eliminates S1 / S2 / S3 under Phase 1 constraints).
- Implementation commit: `d294fb8` ("fix(cli): remove Phase 1 cluster
  init verb").
- Phase 5 reintroduction tracking: GH #81 (`cluster init` + `op
  create` + `op revoke`).
- DELIVER artefacts updated in lockstep:
  `docs/feature/phase-1-control-plane-core/distill/{wave-decisions,walking-skeleton,test-scenarios}.md`,
  `docs/feature/phase-1-control-plane-core/design/wave-decisions.md`,
  `docs/feature/phase-1-control-plane-core/deliver/roadmap.json`,
  and the SSOT scenario mirrors at
  `docs/scenarios/phase-1-control-plane-core/{walking-skeleton,test-scenarios}.md`.
