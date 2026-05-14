# Upstream Changes — service-vip-allocator → workload-kind-discriminator (Slice 06)

**Wave**: DESIGN
**Feature**: service-vip-allocator
**Date**: 2026-05-14
**Author**: Morgan (nw-solution-architect)

## Summary

The DISCUSS wave's Changed Assumption (platform-issued VIPs only,
operator-supplied `vip = Some(...)` rejected) had two possible
resolution paths: parser-level rejection or admission-level
rejection. DESIGN resolves to **admission-level rejection**
(ADR-0049 § 5). This means **the upstream Slice 06 of
`workload-kind-discriminator` requires NO changes** — its spec shape
is preserved verbatim.

## What is preserved (zero change)

| Slice 06 artifact | Status | Reference |
|---|---|---|
| `Listener` struct shape with `vip: Option<ServiceVip>` field | **Unchanged** | ADR-0047 § 4a (`ListenerRow`); `docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md` |
| Parser validation rules for `[[listener]]` block | **Unchanged** | ADR-0047 § 2 validation table |
| `ServiceSpec.listeners: NonEmptyVec<Listener>` | **Unchanged** | ADR-0047 § 1 |
| Forward-compatibility framing in slice-06 R6.1 ("`Option`-shaped field is forward-compatible with both decisions") | **Preserved verbatim** | slice-06 line 132–137 |

The slice-06 author wrote `Option<ServiceVip>` deliberately to be
forward-compatible with either "reject at admission" or "allocate at
runtime". DESIGN picks "allocate at runtime, admission-rejects
operator-supplied `Some`" — both halves of the framing are
honoured.

## What is added (additive only)

### 1. Admission-layer rejection (new, in `overdrive-control-plane`)

```rust
// in overdrive-control-plane::admission (or equivalent)
#[derive(thiserror::Error, Debug)]
pub enum AdmissionError {
    // ... existing variants ...

    /// Operator-supplied `vip = Some(...)` in a Service listener.
    /// VIPs are platform-issued only; operators must remove the
    /// `vip` field from the listener block.
    #[error("listener {listener_idx}: vip is platform-issued; remove the `vip` field from the listener block")]
    VipNotOperatorAssignable { listener_idx: usize },
}
```

Validation runs after parser but before spec-digest compute. Per
AC-06: "No allocator state is mutated and no admission occurs."
The check is a single-pass walk over `spec.listeners` with no I/O.

### 2. `ServiceVip` newtype consolidation (in `overdrive-core`)

The codebase currently has **two `ServiceVip` declarations**:

| Location | Shape | Used by |
|---|---|---|
| `crates/overdrive-core/src/aggregate/workload_spec.rs:360` | `pub struct ServiceVip(pub Ipv4Addr)` | ADR-0047 Slice 06 spec layer |
| `crates/overdrive-core/src/id.rs:647` | `pub struct ServiceVip(std::net::IpAddr)` | other call sites |

This is a latent inconsistency — IPv4-only vs both-families — that
predates this feature. ADR-0049 § 2 consolidates to a **single
canonical declaration** at
`crates/overdrive-core/src/id.rs:647`, retyped to
`pub struct ServiceVip(Ipv4Addr)` (IPv4-only per #167 § Out of
scope; IPv6 is GH #61). The declaration at `workload_spec.rs:360`
is deleted in the same commit; the `Listener.vip` field references
the surviving canonical type.

This is a single-cut consolidation per
`feedback_single_cut_greenfield_migrations.md`. No deprecation
shim, no parallel paths. Mechanical edit; bounded to ~10 use
sites.

**Why consolidate now, not in a separate cleanup PR**: the
allocator's `Token` impl for `ServiceVip` is unambiguous only when
there is one `ServiceVip` type. Picking either declaration mid-PR
creates the inconsistency in reverse. Consolidating in the same PR
is the structurally honest move.

## What stays out of scope

### Slice 06 spec-shape rework

None. The `Option<ServiceVip>` field shape, the `[[listener]]`
TOML structure, and the parser's validation table all stay
verbatim. The `ListenerRow.vip` field semantics evolve from
"operator-pinned or platform-allocated" to "always
platform-allocated, admission rejects `Some(_)`" — but the type
shape is forward-compatible and does not change.

### Parser-layer rejection (rejected alternative)

Parser-level rejection was considered (ADR-0049 § 5 alternative
(a)) and rejected for two reasons:

1. The parser is a structural validator, not a policy layer.
   "Operator may not assign VIPs" is a platform policy that
   future tiers (or future deployment modes) may relax. Encoding
   it in the parser ties policy evolution to parser surface
   evolution.
2. The Slice 06 framing was deliberately forward-compatible.
   Reversing it would require both a slice-06 amendment and a
   migration story for any in-flight parsed specs (no-op today —
   greenfield — but the framing is the load-bearing artifact).

## Verification

- **No edit to `docs/feature/workload-kind-discriminator/slices/slice-06-service-listener-fields.md`** is required by this DESIGN wave. The product owner may verify by diff against the prior commit; the file is untouched.
- **No edit to ADR-0047** is required. ADR-0049 cross-references ADR-0047 in its "Relates to" preamble; ADR-0047 remains the SSOT for the listener spec shape.
- **No new GitHub issues** are created by this DESIGN wave. The DISCUSS wave already determined no deferrals were needed for #167.

## Product owner review

The product owner is invited to confirm:

1. The "no change to Slice 06" framing matches their intent. The
   DISCUSS wave's Changed Assumption (§ Changed Assumptions in
   `docs/feature/service-vip-allocator/discuss/wave-decisions.md`)
   stated "this change is NOT applied directly to
   slice-06-service-listener-fields.md per the task framing and
   the project's deferral discipline." DESIGN honors that framing.
2. The `ServiceVip` consolidation lands in this feature's PR, not
   a separate cleanup PR. The consolidation is necessary to make
   the allocator's Token impl unambiguous; sequencing it separately
   would require a parallel-path period during which two `ServiceVip`
   types coexist, contradicting `feedback_single_cut_greenfield_migrations.md`.

If either is contested, raise during the DESIGN-wave peer review or
between waves; otherwise this back-propagation is the final shape.
