# Slice 06 — Service `[[listener]]` spec shape (port, protocol, optional VIP)

**Outcome**: an operator can declare per-listener `(port, protocol)` and an
optional `vip` on a Service spec; the parser validates triples; the CLI submit
echo and `alloc status` render Listeners honestly. Runtime allocator behaviour
when `vip = None` is OUT OF SCOPE — tracked at #167.

**Stories**: US-08 (Service listener spec shape).

**Folded in**: 2026-05-10 from
[overdrive-sh/overdrive#164](https://github.com/overdrive-sh/overdrive/issues/164)
with user approval. Converged decisions recorded in #164's comment
[#issuecomment-4413120509](https://github.com/overdrive-sh/overdrive/issues/164#issuecomment-4413120509)
and in `wave-decisions.md` § "Fold-in of GH #164".

**Learning hypothesis**: shipping the spec shape now without the runtime
allocator (#167) lets the platform validate operator intent and round-trip
listener triples through submit + alloc status without committing the runtime
to either admission-time rejection or allocator behaviour. Whichever way the
allocator decision lands, the field shape stays `Option`-shaped — forward-
compatible regardless.

## What ships in this slice

- A `Listener` aggregate type at the spec module boundary carrying:
  - `port: NonZeroU16` (parser rejects 0; rendered numerically).
  - `protocol: Proto` — the existing `overdrive-core::Proto` newtype, parsed
    case-insensitively, canonicalised lowercase on render.
  - `vip: Option<ServiceVip>` — `ServiceVip` is a thin newtype over
    `Ipv4Addr`; absent value is `None`. Validation is IPv4 syntactic only at
    this layer.
- TOML deserialisation of `[[listener]]` as a top-level array-of-tables
  alongside `[service]` (NOT nested inside `[service]`).
- Parser validation rules:
  - A Service spec MUST carry at least one `[[listener]]` block.
  - No two `[[listener]]` blocks within a Service may share `(vip, port,
    protocol)`. When both `vip` are `None`, the comparison is on `(port,
    protocol)` only.
  - `protocol` is one of `tcp`/`udp` (case-insensitive); anything else is
    rejected with named guidance ("supported protocols: tcp, udp").
- CLI submit echo (Service kind only) extended with a Listeners section, one
  line per listener: `<vip-or-pending>:<port>/<protocol>`. Pending VIPs render
  as `(vip: pending allocation — see #167)`.
- CLI `alloc status --job <id>` (Service render branch) extended with a
  Listeners section mirroring submit echo semantics.
- `utoipa::ToSchema` derives on `Listener` and `ServiceVip` so
  `cargo openapi-gen` / `cargo openapi-check` continue to pass.
- Property test for `JobSpecInput` ↔ `Job` ↔ TOML/JSON round-trip preservation
  including listener triples in declaration order.

## End-to-end value

- An operator who writes `[[listener]]` blocks under `[service]` gets parser
  validation, a submit echo that mirrors their declared triples, and an
  `alloc status` view that names the same triples post-hoc. They can commit
  Service specs that capture the protocol/port shape today, knowing the
  runtime allocator (#167) will arrive without breaking their TOML.

## Acceptance evidence

- Parser unit tests cover: zero listeners, duplicate triple, unsupported
  protocol, port=0, case-insensitive protocol parsing, mixed-pinned-and-pending
  VIPs.
- A new integration test under
  `crates/overdrive-cli/tests/integration/job_submit_service_listeners.rs` (or
  similar — architect to confirm) submits a Service with two listeners (one
  pinned, one pending) and asserts byte-equality between submit echo and
  `alloc status` listener rendering.
- `cargo openapi-gen` + `cargo openapi-check` both exit 0; generated schema
  includes `Listener` with `port`, `protocol`, optional `vip`.
- Property test asserts `JobSpecInput` round-trip through TOML/JSON/Job.
- Test runner: every acceptance test invocation is `cargo xtask lima run --
  cargo nextest run …` per `.claude/rules/testing.md`.

## Effort estimate (advisory)

~1.5 days. Spec types + parser + uniqueness validation + CLI echo render +
alloc status render + OpenAPI derives + property test. Comparable to Slice 02
in scope.

## Carpaccio shape — single slice, defended

This slice ships seven moving parts: (1) listener types, (2) parser
deserialisation, (3) parser validation rules, (4) submit echo render, (5)
alloc status render, (6) OpenAPI derives, (7) property test. Borderline thick.
Defence for keeping it whole rather than splitting:

- The listener types ARE the new abstraction; parser and renders without the
  types are nonsensical, just as Slice 01's parser is the abstraction
  every later slice depends on. Splitting "parser only" from "render only"
  would leave one half landed and the other unable to demonstrate end-to-end
  value (an operator with a parsed-but-not-rendered listener has gained
  nothing visible).
- The OpenAPI derives are mechanical — adding `#[derive(ToSchema)]` is a
  single-line change per type. Splitting that out as its own slice would be
  pure overhead.
- The property test is the round-trip check that catches drift between TOML,
  JSON, and the `Job` aggregate. It must land with the types or it cannot
  exist.

If the architect later determines the alloc status render extension is
non-trivial (e.g. requires denormalising listener triples onto
`AllocStatusRow`), splitting at the AllocStatusRow boundary is the natural
fault line — Slice 06a (parser + types + submit echo + OpenAPI + property
test) and Slice 06b (alloc status listeners section). DISCUSS leaves that
decision to DESIGN since the persistence shape is theirs to pin.

## Dependencies

- **Hard**: Slice 01 (parser kind discriminator). The `WorkloadKind::Service`
  variant must exist before listeners can hang off it.
- **Hard**: Slice 04 (Service preservation) for the `format_running_summary`
  / Service render branch the submit echo extension extends.
- **Soft**: Slice 03 (alloc status Job render). Slice 03 builds the kind-
  aware render machinery that Slice 06 extends with a Listeners section for
  Service kind. Could ship in either order if the render machinery is built
  generically; architect to confirm.
- **Independent of**: Slice 02 (Job submit terminal — Job kind, no listeners
  in this feature) and Slice 05 (Schedule parsing — Schedule kind also has no
  listeners in this slice; if Schedule eventually needs listeners, it will be
  a separate fold-in).

## Reference class

Closest analogue: Slice 01 (parser-and-validation slice with downstream
render echoes). Slice 06 is roughly 1.5× Slice 01's scope because it adds the
alloc status render extension and the property test that Slice 01 did not
need.

## Risks

- **R6.1**: If #167 (VIP allocator) lands with a different field shape than
  `Option<ServiceVip>`, downstream rework is needed. **Mitigation**: the
  `Option`-shaped field is forward-compatible with both decisions ("reject at
  admission" → `None` is a parser error in a future ADR; "allocate at
  runtime" → `None` is the trigger for the allocator). The spec field stays
  the same shape either way.
- **R6.2**: The `Backend` type in `crates/overdrive-sim/src/adapters/
  dataplane.rs` is the dataplane's destination-address record. Naming this
  slice's section `[[listener]]` (per #164's converged decision) avoids the
  collision.
- **R6.3**: `utoipa::ToSchema` for `Option<ServiceVip>` may need explicit
  schema attributes if the auto-generated schema does not match the runtime
  serde shape. **Mitigation**: `cargo openapi-check` is the gate — if it
  fails, the architect adds the explicit schema annotation.

## DoR fit

Listener spec shape is a coherent slice with end-to-end operator value. The
runtime allocator (#167) is a separate primitive whose shape this slice does
not constrain. Carpaccio carve passes the six taste tests:

1. **Vertical end-to-end?** Yes — operator writes TOML, sees parsed echo,
   inspects `alloc status`.
2. **Demonstrable in a single session?** Yes — write spec, submit, observe
   echo, run alloc status.
3. **Independent of later slices?** Yes — does not depend on Slices 02 or
   05; depends on 01 and 04 which precede it.
4. **Right-sized?** ~1.5 days, 9 UAT scenarios. At the upper edge but the
   scenarios are tight and trace 1:1 to AC.
5. **One bounded context?** Yes — CLI/control-plane spec layer, same context
   as Slices 01–05.
6. **Walking-skeleton fit?** N/A — this feature evolves a landed walking
   skeleton.

## DESIGN-wave handoff notes

- ADR-0031 amendment for the `[exec]` block placement — extend to cover the
  new top-level `[[listener]]` array-of-tables.
- A new ADR may be warranted for the `Listener` aggregate; alternatively, the
  workload-kind-discriminator ADR (working title from research R4) may grow
  a "Service listener fields" section. Architect's call.
- The runtime allocator design is #167's concern; this slice's ADR work
  should NOT prescribe runtime semantics for `vip = None`.
