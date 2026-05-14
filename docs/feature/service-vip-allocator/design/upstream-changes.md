# Upstream Changes — service-vip-allocator → workload-kind-discriminator (Slice 06)

**Wave**: DESIGN
**Feature**: service-vip-allocator
**Date**: 2026-05-14 (amended same day — see § Amendment)

## Summary

The DISCUSS wave's Changed Assumption (platform-issued VIPs only,
operator-supplied `vip = Some(...)` not honored) had two possible
resolution paths: parser-level rejection (remove the field) or
admission-level rejection (preserve the field, refuse `Some(_)` at
admission time).

**Initial DESIGN resolution (2026-05-14, withdrawn)**: admission-level
rejection. Field preserved.

**Amended resolution (2026-05-14, current)**: **parser-level removal
of the `vip` field on `Listener`**. Per
`.claude/rules/development.md` § "Type-driven design" → "make
invalid states unrepresentable", the prior admission-level
rejection defended a state the type system can exclude
structurally; with operator-pinned VIPs explicitly decided against,
the field is meaningless on the operator spec and is deleted.

This is a real spec-shape back-propagation against slice-06 of
`workload-kind-discriminator`, single-cut per
`feedback_single_cut_greenfield_migrations.md`. Slice-06's
already-shipped tests delete in the same commit that lands the
field removal.

## Amendment

Per user direction citing `.claude/rules/development.md` →
"Type-driven design — make invalid states unrepresentable". Detail
in ADR-0049 § 5 (amended) + § 5a (new — placement of the assigned
VIP).

## Per-file changes against slice-06

The product owner does NOT modify
`docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md`
directly during this DESIGN wave. This document is the
specification of what the implementation crafter for
`service-vip-allocator` will edit when they land the feature. The
slice-06 brief itself is untouched.

Quoted line numbers are against
`docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md`
at the commit that produced this DESIGN wave (verify via
`git diff` against the prior commit at landing time).

### 1. `Listener` aggregate type — remove the `vip` field

Slice-06 brief lines 25–31 specify:

> A `Listener` aggregate type at the spec module boundary carrying:
>
> - `port: NonZeroU16` (parser rejects 0; rendered numerically).
> - `protocol: Proto` — the existing `overdrive-core::Proto` newtype,
>   parsed case-insensitively, canonicalised lowercase on render.
> - `vip: Option<ServiceVip>` — `ServiceVip` is a thin newtype over
>   `Ipv4Addr`; absent value is `None`. Validation is IPv4 syntactic
>   only at this layer.

**Post-amendment shape**:

```rust
pub struct Listener {
    pub port:     NonZeroU16,
    pub protocol: Proto,
    // vip field removed — platform-issued only; see ADR-0049 § 5.
}
```

The parser uses `#[serde(deny_unknown_fields)]` (or the TOML
deserializer's equivalent) so an operator-supplied `vip = "..."`
fails at parse time with a typed error naming the field and
guiding the operator: "the `vip` field is not operator-assignable;
the platform allocates Service VIPs automatically. Remove it from
the `[[listener]]` block." (Implementation note: the exact
error-message wording is the crafter's call; the named-guidance
quality is the load-bearing property — operators must know what to
do.)

### 2. Listener uniqueness rule — simplify

Slice-06 brief lines 36–38 specify:

> No two `[[listener]]` blocks within a Service may share `(vip, port,
> protocol)`. When both `vip` are `None`, the comparison is on `(port,
> protocol)` only.

**Post-amendment rule**:

> No two `[[listener]]` blocks within a Service may share `(port,
> protocol)`.

The "when both `vip` are `None`" branch is deleted — without a
`vip` field, the comparison is always on `(port, protocol)`.

### 3. CLI submit echo render — drop the per-listener VIP slot

Slice-06 brief lines 41–43 specify:

> CLI submit echo (Service kind only) extended with a Listeners section,
> one line per listener: `<vip-or-pending>:<port>/<protocol>`. Pending
> VIPs render as `(vip: pending allocation — see #167)`.

**Post-amendment shape**:

> CLI submit echo (Service kind only) extended with a Listeners
> section, one line per listener: `<port>/<protocol>`. The
> allocator-assigned VIP renders **at the Service level**, not
> per-listener — one assigned VIP per Service, shared across all
> of its listeners.

The Service-level VIP render consults `ServiceVipAllocator::get(&spec_digest)`
at echo time. Implementation surface (rough — final decision is the
crafter's): a single `assigned_vip:` line above the Listeners
section, or a Service header line that includes `vip = <Ipv4Addr>`
alongside the existing Service fields. The "pending allocation"
wording from the prior shape is dropped — by AC-01, the assigned
VIP is known synchronously at submit time and is never pending in
the echo path.

### 4. `alloc status --job <id>` Service render branch — same shape change

Slice-06 brief lines 44–45 specify:

> CLI `alloc status --job <id>` (Service render branch) extended
> with a Listeners section mirroring submit echo semantics.

Inherits the same shape changes from § 3 above: listener lines
become `<port>/<protocol>`; assigned VIP renders at Service level
via the same `ServiceVipAllocator::get(&spec_digest)` consult.
Note that `alloc status --service <id>` (a render path AC-06 of
#167 names) is the same render path here — both `--job` and
`--service` flags on `alloc status` route to the Service render
branch.

### 5. `utoipa::ToSchema` derives — schema delta

Slice-06 brief lines 46–47 specify:

> `utoipa::ToSchema` derives on `Listener` and `ServiceVip` so
> `cargo openapi-gen` / `cargo openapi-check` continue to pass.

The `Listener` schema no longer carries a `vip` field; `ServiceVip`
remains a derived schema (used by the allocator's persisted state
codec and by `Dataplane::update_service`'s VIP parameter — see
ADR-0049 § 2). `cargo openapi-gen` is rerun; the OpenAPI golden
fixture for `Listener` updates in the same commit; `cargo
openapi-check` exits 0 on the new shape.

### 6. Already-shipped slice-06 tests — delete and replace

Slice-06 brief lines 60–63 specify parser unit test coverage for:

> zero listeners, duplicate triple, unsupported protocol, port=0,
> case-insensitive protocol parsing, mixed-pinned-and-pending VIPs.

**The "mixed-pinned-and-pending VIPs" test deletes.** It defends a
state the new shape makes structurally impossible. Per
`.claude/rules/development.md` § "Deletion discipline" the test
deletes in the same commit as the field removal — do not salvage
it by rewriting it to test something else.

Slice-06 brief lines 64–68 specify an integration test:

> A new integration test under
> `crates/overdrive-cli/tests/integration/job_submit_service_listeners.rs`
> (or similar — architect to confirm) submits a Service with two
> listeners (one pinned, one pending) and asserts byte-equality
> between submit echo and `alloc status` listener rendering.

**The one-pinned-one-pending integration test deletes** for the
same reason. The byte-equality property is genuinely valuable and
SHOULD be preserved in a new test — but the new test submits a
Service with two listeners on different `(port, protocol)` tuples
(no `vip` field), asserts byte-equality between submit echo and
`alloc status` listener-section rendering, and additionally
asserts that the Service-level VIP render is byte-equal between
the two paths. Written from scratch, not salvaged.

Slice-06 brief lines 70–71 specify:

> Property test asserts `JobSpecInput` round-trip through TOML/JSON/Job.

**The property test updates** — the round-trip generator now
produces `Listener { port, protocol }` (no `vip` field). The
round-trip property itself is preserved verbatim; only the
generator's surface changes.

New tests defending the new shape (parser rejects `vip = "..."`
with `unknown field` + named guidance; uniqueness on `(port,
protocol)`) are written from scratch and land in the same commit.

### 7. R6.1 risk mitigation — moot

Slice-06 brief lines 132–137 frame R6.1:

> R6.1: If #167 (VIP allocator) lands with a different field shape
> than `Option<ServiceVip>`, downstream rework is needed.
> Mitigation: the `Option`-shaped field is forward-compatible with
> both decisions ("reject at admission" → `None` is a parser error
> in a future ADR; "allocate at runtime" → `None` is the trigger
> for the allocator). The spec field stays the same shape either way.

**The mitigation argument no longer applies.** With the field
removed, the "Option-shaped field is forward-compatible" framing
is moot — the risk resolves by deletion, not by mitigation. The
slice-06 author's deliberate forward-compatible framing is
acknowledged; the project's evolved type-driven-design discipline
(this amendment) supersedes it.

The risk-management lesson is preserved in `wave-decisions.md`:
forward-compatibility for a feature explicitly decided against is
the deferral-without-issue shape, and the right move is removal,
not future-proofing.

## `ServiceVip` newtype consolidation (unchanged)

The codebase currently has **two `ServiceVip` declarations**:

| Location | Shape | Used by |
|---|---|---|
| `crates/overdrive-core/src/aggregate/workload_spec.rs:360` | `pub struct ServiceVip(pub Ipv4Addr)` | ADR-0047 Slice 06 spec layer (now: removed alongside the `Listener.vip` field, since `Listener` no longer references this type) |
| `crates/overdrive-core/src/id.rs:647` | `pub struct ServiceVip(std::net::IpAddr)` | other call sites |

This is a latent inconsistency — IPv4-only vs both-families — that
predates this feature. ADR-0049 § 2 consolidates to a **single
canonical declaration** at
`crates/overdrive-core/src/id.rs:647`, retyped to
`pub struct ServiceVip(Ipv4Addr)` (IPv4-only per #167 § Out of
scope; IPv6 is GH #61). The declaration at `workload_spec.rs:360`
is deleted in the same commit. Post-amendment, the consolidation
is **even cleaner**: `Listener` no longer references `ServiceVip`
at all, so the only remaining references are the allocator's own
codec (`AllocatorTokenBytes::ServiceVip(Ipv4Addr bytes)`), the
downstream `Dataplane::update_service(_, vip: ServiceVip, _)`
parameter, and the `ServiceMapHydrator`'s now-direct allocator
consult.

This is a single-cut consolidation per
`feedback_single_cut_greenfield_migrations.md`. No deprecation
shim, no parallel paths. Mechanical edit; bounded use sites.

## What stays out of scope

### Direct edits to slice-06's brief

This DESIGN wave does NOT modify
`docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md`
directly. The brief itself stays unchanged; the implementation
crafter dispatched against #167 edits slice-06's brief alongside
the codebase changes when the feature lands, so the brief and the
code stay in lockstep (single-cut migration).

### Parser-layer rejection that *preserves* the field

This was the prior DESIGN resolution (admission-level rejection,
field preserved). It is now rejected — see ADR-0049 § 5 (amended)
considered-alternatives table option (b).

## Verification

- **Edit to `docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md` is NOT required** by this DESIGN wave artifact. The implementation crafter will edit slice-06's brief alongside the codebase changes when landing the feature. The product owner may verify by diff against the prior commit during the implementation review.
- **No edit to ADR-0047** is required. ADR-0049 cross-references ADR-0047 in its "Relates to" preamble; ADR-0047's `ListenerRow.vip: Option<ServiceVip>` field reference is annotated as amended by ADR-0049 § 5.
- **No new GitHub issues** are created by this DESIGN wave. The DISCUSS wave already determined no deferrals were needed for #167; the parser-level removal amendment introduces no new deferrals.

## Product owner review

The product owner is invited to confirm:

1. **Type-driven-design principle takes precedence over forward-compatibility framing.** The slice-06 author wrote `Option<ServiceVip>` deliberately to be forward-compatible with both "reject at admission" or "allocate at runtime". The project's evolved discipline (this amendment cites `.claude/rules/development.md` § "Type-driven design" → "make invalid states unrepresentable") removes the field rather than refining the constraint at admission. Operator-pinned VIPs are explicitly out of scope for the platform; defending future-compatibility with that non-feature is the deferral-without-issue shape CLAUDE.md § "Deferrals require GitHub issues" forbids.

2. **Slice-06's already-shipped tests delete in the same commit as the field removal.** Per `feedback_single_cut_greenfield_migrations.md` + `.claude/rules/development.md` § "Deletion discipline" — no parallel paths, no deprecation period, no salvage-by-rewriting. The mixed-pinned-and-pending parser test, the one-pinned-one-pending integration test, and the property test's listener-triple generator all change shape with the field. New tests defending the new shape land in the same commit.

3. **The assigned VIP lives in the allocator's own persisted memo** (ADR-0049 § 5a, Option C of three). Not on `Service`/`Job` (Option A — rejected as putting an operator-shape field that's not operator-set on the aggregate). Not in observation (Option B — rejected as introducing a second source of truth and requiring synchronous observation-write at admission). The render-time consult `ServiceVipAllocator::get(&spec_digest)` is the single source of truth; the `Job` aggregate stays purely operator-input.

4. **The `ServiceVip` consolidation lands in this feature's PR, not a separate cleanup PR.** Unchanged from the prior resolution — the consolidation is necessary to make the allocator's `Token` impl unambiguous; sequencing it separately would require a parallel-path period during which two `ServiceVip` types coexist.

If any is contested, raise during the DESIGN-wave peer review or
between waves; otherwise this back-propagation is the final shape.
