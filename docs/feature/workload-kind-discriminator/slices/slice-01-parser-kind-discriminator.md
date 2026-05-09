# Slice 01 — Spec parser accepts `[service]` / `[job]` discriminator

**Outcome**: an operator can write a Service spec OR a Job spec in TOML, the parser
recognises the kind by section presence, and mixed-kind specs are rejected with named
guidance.

**Stories**: US-01 (parser kind discriminator), US-06 (anti-pattern grep gate for
`"live"`), US-07 (`examples/coinflip.toml` migration).

**Learning hypothesis**: introducing the `WorkloadKind` enum at the parser boundary as
the new abstraction unblocks every downstream change without breaking the existing
walking-skeleton submit path. We learn whether the section-as-discriminator shape (vs.
internally-tagged `kind = "..."`) matches operator intuition by inspecting the parser-
error feedback during exploratory testing.

## What ships in this slice

- A `WorkloadKind` (or equivalent name — architect to confirm in DESIGN) enum at the
  spec-parser boundary in the CLI library, with three variants: `Service { ... }`,
  `Job { ... }`, `Schedule { ... }`.
- TOML deserialisation that recognises kind by section presence:
  - `[service]` alone → `Service`.
  - `[job]` alone → `Job`.
  - `[job] + [schedule]` → `Schedule`.
  - Anything else → typed parse error.
- Validation rules:
  - Exactly one of `[service]` / `[job]` is required.
  - `[schedule]` is only valid alongside `[job]`, never `[service]`.
  - `[exec]` and `[resources]` are required for all kinds at the top level.
  - `[schedule]` requires a `cron` field (string; semantic parsing is Slice 05).
- Parse error messages:
  - Name the offending sections explicitly.
  - Suggest the corrective action ("exactly one of [service] or [job] is required").
- Migration of `examples/coinflip.toml` from the legacy flat shape to `[job]` — single
  cut, no compat shim per `feedback_single_cut_greenfield_migrations.md`.
- Anti-pattern grep gate: a CI check (or an `xtask` lint) that fails if the literal
  string `"live"` appears as a hard-coded duration in CLI render code. The gate is
  introduced in this slice so subsequent slices cannot regress to the bug shape.

## End-to-end value

- An operator who writes `[service]` and `[exec]` and `[resources]` gets a
  `WorkloadSpec::Service` (no submit semantics yet — that's still the legacy code path
  in this slice; full Service-side wiring is Slice 04 vocabulary preservation).
- An operator who writes `[service]` AND `[job]` gets a typed parser error naming both
  sections within 50ms p95.
- An operator can write a `[job]`-shaped `examples/coinflip.toml` that parses (the
  submit path still uses the legacy semantics in this slice — Slice 02 changes that).

## Acceptance evidence

- Parser unit tests cover all three valid kinds + the invalid combinations from the
  feature file.
- Migration of `examples/coinflip.toml` is verified by submitting the new file and
  observing the parser accepts it.
- Anti-pattern grep gate fails on a deliberately-introduced regression test.

## Effort estimate (advisory)

~1 day. A single engineer can land parser + validation + tests + the migration in
one focused day. The grep gate is small (an `xtask` test or a `dst-lint`-style
pattern) and adds <2 hours.

## Risks

- `serde` untagged-enum + section-presence pattern has some boilerplate; the architect
  may prefer a custom `Deserialize` impl for clearer error messages.
- The parser must produce parse errors that name the OFFENDING SECTIONS (not just
  "deserialize failed"). This requires per-field deserialisation rather than a blanket
  `serde(deny_unknown_fields)`.

## DoR fit

This slice owns the new abstraction (`WorkloadKind`) — Slices 02–05 build on it. Slice
01 must ship first or the rest are blocked.
