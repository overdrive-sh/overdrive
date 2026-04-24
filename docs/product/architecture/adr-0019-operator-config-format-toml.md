# ADR-0019 — Operator config format: TOML for `~/.overdrive/config`, superseding ADR-0010 §R2

## Status

Accepted. 2026-04-24. Supersedes ADR-0010 §R2 only. ADR-0010 R1, R3,
R4, R5 remain in force; the ephemeral-CA + base64-embedded-trust-triple
+ no-`--insecure` + defer-to-Phase-5 decisions are unchanged.

## Context

ADR-0010 §R2 (Accepted 2026-04-23) specified YAML as the on-disk format
for `~/.overdrive/config`, citing "same shape as `~/.talos/config` and
`~/.kube/config`" as the rationale. The Talos bootstrap research
(`docs/research/security/talos-bootstrap-tls-strategy-comprehensive-research.md`,
§R2 Divergence check) inherited YAML from the Talos reference point
without interrogating the format itself — the research question was
*what goes in the file*, not *what the file is serialized as*.

A subsequent architect brief
(`.context/toml-config-supersession-brief.md`, 2026-04-24) reopened the
decision on the grounds that "same shape" conflates two orthogonal
concerns:

1. **Operator ergonomics** — the conceptual shape (contexts,
   `current-context`, multi-cluster semantics, base64-embedded trust
   triple). This is the kubectl/talosctl muscle memory operators
   actually carry over.
2. **Serialization syntax** — YAML vs TOML vs JSON. An implementation
   detail visible to operators only during hand-edits.

Ergonomics transfer through the shape, not the syntax. A TOML file with
identical field names, identical semantics, identical base64 PEM
embedding, and identical `current-context` pointer reads as "a
kubeconfig in TOML" to any kubectl-fluent operator.

Three forces push toward reopening:

- **Consistency.** Every other operator-facing config in the project is
  TOML: `Cargo.toml` + `package.metadata.overdrive.crate_class`
  (ADR-0003), whitepaper §4 `[cluster]` / `[cluster.observation]`
  blocks, whitepaper §6 `[job]` / `[job.microvm]` / `[job.security]`,
  whitepaper §7 `[node.gateway.acme]`, whitepaper §11 `[[routes]]`
  with `[routes.middleware]`, whitepaper §23 schematics (content-hashed
  via `SchematicId`, ADR-0002). `~/.overdrive/config` is the only
  operator-facing YAML hole.
- **Ecosystem maturity (design principle 7).** `serde_yaml` was
  archived by its maintainer in 2024-03. The community successor
  `serde_yml` is a fork with uncertain long-term governance. The
  `toml` crate is co-developed alongside Cargo and is one of the most
  actively maintained serde backends.
- **YAML 1.1 footguns.** The Norway problem (`NO` → `false`), octal
  coercion of leading-zero integers, sexagesimal parsing of
  `12:34:56`, indentation-sensitive misparse. Irrelevant for the
  base64 PEM fields in the current schema; a loaded gun for any
  future field carrying a two-letter country code, job name, or
  unquoted string-like value. TOML rejects these by construction.

## Decision

**On-disk format of `~/.overdrive/config` is TOML.** The YAML shape
specified in ADR-0010 §R2 is superseded. Every operator-facing
concept — field names, context model, `current-context` pointer,
base64 PEM embedding of the trust triple, `OVERDRIVE_CONFIG` env var
path override, no env-carried cert material — is preserved bit-for-bit.

### Canonical shape

```toml
current-context = "local"

[[contexts]]
name     = "local"
endpoint = "https://127.0.0.1:7001"
ca       = "<base64-encoded PEM CA cert>"
crt      = "<base64-encoded PEM client leaf cert>"
key      = "<base64-encoded PEM client leaf private key>"
```

Array-of-tables (`[[contexts]]`) is idiomatic TOML for a
`contexts: list<record>` field. One-to-one with the superseded YAML
shape.

### What stays from ADR-0010

- **R1** — ephemeral in-process CA at `overdrive cluster init`.
  Unchanged.
- **R3** — multi-SAN server leaf cert (`IP:127.0.0.1`, `IP:::1`,
  `DNS:localhost`, `DNS:<hostname>`). Unchanged.
- **R4** — no `--insecure` flag. Unchanged.
- **R5** — defer rotation, revocation, roles, persistence to Phase 5.
  Unchanged. Phase 5 additions (operator SPIFFE IDs in the client
  cert SAN, `acceptedCAs` multi-CA window, Corrosion-gossiped
  revocation) remain additive on top of the TOML shape — no
  file-format migration at the Phase 1 → Phase 5 boundary.
- **Overdrive-specific divergence from Talos** (role-in-cert rejected
  in favour of Phase 5 SPIFFE URI SANs) — unchanged. This ADR adds a
  second, orthogonal divergence: syntax. Both are explicit,
  documented, and rationale-backed.

### What changes

- On-disk syntax: YAML → TOML.
- CLI loader: `serde_yaml` / `serde_yml` → `toml` (already in the
  workspace transitively via Cargo). Design principle 7 strengthened,
  not weakened.
- Test scenarios and acceptance tests that reference "parses as YAML"
  update to "parses as TOML" (distill scenarios, tls_bootstrap tests,
  acceptance). Per user memory
  `feedback_single_cut_greenfield_migrations`, the change lands as a
  single PR: no YAML fallback, no grace period, no feature flag, no
  deprecation window. YAML support is deleted in the same commit TOML
  support is added.

### Multi-context forward-compat

Phase 5 federated multi-region config (whitepaper §4 *Multi-Region
Federation*) remains expressible in TOML without ceremony. Prior art:
Cargo's `[profile.*]` stacking and `[target.'cfg(...)'.dependencies]`
handle deeper nesting than `~/.overdrive/config` is projected to
require. Array-of-tables with nested inline tables covers every
federated-context shape currently sketched in the whitepaper.

## Considered alternatives

### Alternative A — Keep YAML (ADR-0010 §R2 status quo)

**Rejected.** The "same shape as kubeconfig" argument fails on closer
inspection because the field set already diverges from kubeconfig
(no `users` / `clusters` split, no `insecure-skip-tls-verify`, no
`auth-provider`). The "YAML means kubectl can read it" claim was
never true — operators use `overdrive` to read the file, not
`kubectl`, and the divergent schema means kubectl-ecosystem tooling
(`kubectl config view`, k9s context switchers, credential helpers)
cannot consume the file regardless of syntax. Holding YAML to avoid
non-existent interop is cost with no corresponding benefit. Add the
`serde_yaml` archival status and YAML 1.1 footguns, and the trade
inverts.

### Alternative B — JSON

**Rejected.** JSON is the strictest of the three on types and parser
behaviour, which is attractive, but:

- No comments. Operators hand-editing a trust triple cannot annotate
  context provenance, intended lifetime, or ops-handoff notes.
  Kubeconfig-equivalent files are meant to be read by humans as well
  as machines.
- Every other config in the project is TOML. JSON would make
  `~/.overdrive/config` the only JSON file in the operator-facing
  surface, trading one kind of odd-one-out for another.
- Canonical serialization requirements (ADR-0002's RFC 8785 JCS
  discipline for content-addressed IDs) are unrelated to CLI config.
  The canonicalization story argues for JCS *where hashing is
  involved*, not for JSON as a human-editable config format.

### Alternative C — TOML (chosen)

Accepted for the reasons in Context. The brief's case is strong on
consistency, ecosystem maturity, and footgun elimination; none of the
kubectl-interop counter-arguments survive inspection.

## Consequences

### Positive

- **Single config language across the project.** Operators learn TOML
  once for `Cargo.toml`, cluster config, job specs, schematics, and
  `~/.overdrive/config`. Cognitive load drops rather than rises.
- **Design principle 7 strengthened.** Moving off an archived serde
  backend (`serde_yaml`) onto an actively maintained, Cargo-adjacent
  one (`toml`) removes an ecosystem-maturity risk ADR-0010 did not
  catch.
- **YAML 1.1 footguns eliminated by construction.** No Norway
  problem, no octal coercion, no sexagesimal surprise, no
  indentation-sensitive silent misparse. TOML's strict types and
  explicit quoting close each of these.
- **PEM-in-base64 renders cleanly.** TOML's explicit `key = "value"`
  form with required quotes is unambiguous. YAML's block-scalar
  folding (`|`, `>`, `|-`, `>+`) affords multi-line-formatting
  footguns that were never needed but were syntactically available.
- **Phase 5 forward-compat preserved.** Multi-context, multi-cluster,
  `acceptedCAs`, operator SPIFFE IDs in the SAN — all additive on the
  TOML shape, same as they would have been on the YAML shape. No
  second file-format migration at the Phase 5 boundary.

### Negative

- **Second divergence from Talos/kubeconfig reference points.** Role
  encoding (ADR-0010) was the first; syntax is the second. Both are
  explicit, documented, and rationale-backed. Operators familiar with
  `~/.talos/config` encounter "same shape, TOML" rather than
  "identical file."
- **One-time migration cost on Phase 1 tests and scenarios.** The CLI
  loader is ~100 LoC of serde plumbing and a bounded set of test
  fixtures. Phase 1 is still walking-skeleton; the cost is
  time-bounded and does not recur.

### Quality-attribute impact

- **Maintainability — modifiability**: net positive. One config
  language across the project; one actively maintained serde backend.
- **Maintainability — analysability**: net positive. Strict-type TOML
  parsing surfaces malformed input as loud parse errors rather than
  silent coercions. No YAML 1.1 ambiguities for reviewers to carry
  in their head.
- **Security — authenticity**: unchanged. The trust triple and its
  validation semantics are identical.
- **Usability — operability**: net positive in aggregate. Minor cost
  on kubectl-reflexive operators (handled by documentation); larger
  benefit on consistency with the rest of the project's operator
  surface.
- **Portability — ecosystem fit**: net positive. Rust-ecosystem
  operators (the core demographic per user-background memory) meet
  TOML daily in Cargo and cargo-* tooling.

### Enforcement

- The CLI loader consumes `~/.overdrive/config` through the `toml`
  crate only. No `serde_yaml` / `serde_yml` dependency in the
  `overdrive-cli` `Cargo.toml`. A dependency-graph check in CI
  asserts absence; violating the rule fails the build. (The existing
  dst-lint gate handles `core`-class crates per ADR-0003; this is an
  analogous but separate check at the CLI crate boundary.)
- The ADR-0010 rustls enforcement — no `DangerousClientConfigBuilder`,
  no `dangerous_accept_any_server_cert`, grep-gate in CI, compile-fail
  test — is unchanged. The format swap does not touch the TLS trust
  posture.
- Round-trip proptest on the TOML schema per
  `.claude/rules/testing.md` *Newtype completeness / roundtrip*
  discipline: generate arbitrary valid configs, serialize → parse →
  assert bit-equivalent. The existing YAML round-trip test is the
  direct analog and converts one-to-one.
- The file-loader rejection of any context missing `ca` / `crt` /
  `key` (ADR-0010 §Enforcement, last bullet) is unchanged in
  behaviour; the rejection surface moves from YAML parse errors +
  serde `deny_unknown_fields` to TOML parse errors + `#[serde(deny_unknown_fields)]`.

## Open questions resolved inline

- **Kubernetes-ecosystem interop.** No planned external
  kubeconfig-consuming tooling reads `~/.overdrive/config`. The field
  set already diverges from kubeconfig (no `users`/`clusters` split,
  no `insecure-skip-tls-verify`, no `auth-provider`); kubectl cannot
  parse Overdrive's file as a kubeconfig regardless of syntax.
  Resolved: no interop story to preserve.
- **Phase 5 multi-region federation config shape.** No TOML
  expressibility constraint. Whitepaper §4 federation already uses
  `[cluster]` + `[cluster.observation]` TOML blocks; the operator
  config's forward-compat surface is a subset of what Cargo's
  `[profile.*]` / `[target.'cfg(...)']` nesting already proves
  tractable. Resolved: no format-driven limit on Phase 5 shape.
- **Domain boundary — Phase 1 TLS bootstrap vs operator-identity.**
  The brief treats them as one context; this ADR does likewise. The
  config-format decision is a property of the operator config file,
  which is the shared artifact between the Phase 1 TLS bootstrap
  (ADR-0010) and the Phase 5 operator-identity story (whitepaper §8).
  Supersession of ADR-0010 §R2 alone — rather than a new
  cross-cutting ADR — is the minimum-surface decision that keeps the
  history legible. Resolved: stays in the Phase 1 TLS bootstrap
  lineage.

## References

- `docs/product/architecture/adr-0010-phase-1-tls-bootstrap.md` —
  §R2 is superseded by this ADR; R1, R3, R4, R5 are unchanged.
- `docs/research/security/talos-bootstrap-tls-strategy-comprehensive-research.md`
  §R2 — the research finding that established the kubeconfig-shape
  ergonomics; its "Divergence check" line about `serde_yaml` is
  specifically the element this ADR revisits.
- `docs/whitepaper.md` §4 (TOML `[cluster]` / `[cluster.observation]`
  examples), §6 (TOML `[job]` / `[job.microvm]` / `[job.security]`),
  §8 (Operator Identity and CLI Authentication — the semantic
  surface unchanged), §11 (TOML `[[routes]]`), §23 (TOML schematics
  content-hashed via `SchematicId`).
- `docs/product/architecture/adr-0002-schematic-id-canonicalisation.md`
  — ADR-0002 governs canonical hashing of TOML schematics. Relevant
  as prior art for TOML as a canonical-form-friendly operator-facing
  format.
- `docs/product/architecture/adr-0003-core-crate-labelling.md` —
  `package.metadata.overdrive.crate_class` is the other TOML metadata
  surface operators edit; consistency argument rests partly on this
  ADR.
- `.context/toml-config-supersession-brief.md` — the architect brief
  that reopened the decision.
- User memory `feedback_single_cut_greenfield_migrations` — governs
  the zero-fallback migration shape: no YAML support, no feature
  flag, no grace period, delete-old-and-land-new in the same PR.
- User memory `project_cli_auth` — Phase 5 operator-identity shape
  (SPIFFE IDs, 8h TTL, Corrosion-gossiped revocation) that must
  remain additive over the Phase 1 format; preserved.
