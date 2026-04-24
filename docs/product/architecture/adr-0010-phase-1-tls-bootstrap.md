# ADR-0010 — Phase 1 TLS bootstrap: ephemeral in-process CA, embedded trust triple in `~/.overdrive/config`

## Status

Accepted. 2026-04-23. **§R2 superseded 2026-04-24 by ADR-0019**
(on-disk format YAML → TOML; every other aspect of R2 — field names,
context model, `current-context` pointer, base64 PEM embedding,
`OVERDRIVE_CONFIG` env override, kubeconfig-shape ergonomics,
Phase 5 forward-compat — preserved bit-for-bit). R1, R3, R4, R5
remain in force.

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

### R1 — Ephemeral in-process CA at `overdrive cluster init`

On first `overdrive cluster init` (or its Phase 1 equivalent entry
point — the server binary's startup path), the binary generates
in-memory:

- A self-signed CA (P-256, `rcgen` — already in workspace).
- A server leaf certificate signed by that CA, presented on
  `:7001`.
- A client leaf certificate signed by the same CA, handed to the
  operator through `~/.overdrive/config`.

The CA private key lives in process memory only. No persistence.
Process stop discards the CA. Re-starting `cluster init` (or
`--force`) re-mints everything.

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

### R4 — No `--insecure` flag

No CLI flag bypasses server-cert verification. There is no pre-PKI
window (the CA is minted before `bind()` is called), so the flag
would have nothing to justify its existence. Recovery on lost client
cert is `overdrive cluster init --force`, not a verification-skip.

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
