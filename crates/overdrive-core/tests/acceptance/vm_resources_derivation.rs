//! S-VM-73 — vCPU derivation rounds up and floors at one (Tier-1,
//! `@property` `@in-memory`; ADR-0082 §D2 "derived from `cpu_milli`, floor
//! 1", US-VM-5 feature-delta §§1683-1780).
//!
//! Driving port: the pure `overdrive_core::vm::config::vcpus_for` function.
//! A pure function IS its own driving port — calling it directly in the
//! test IS port-to-port testing at the domain layer (per
//! `nw-tdd-methodology` § "Hexagonal Architecture Testing Strategy").
//!
//! The derivation RULE is DESIGN's and fixed (US-VM-5 Technical Notes):
//! `max(1, round_up(cpu_milli / 1000))`, saturated into the `NonZeroU8`
//! that ADR-0082 §D2 pins as `VmConfig.vcpus` — so "never zero" is a type
//! invariant, and the floor of one means a sub-core request yields ONE
//! vCPU rather than refusing (a VM cannot have a fractional CPU).
//!
//! **Mutation target** (`.claude/rules/testing.md` § "Mutation testing
//! (cargo-mutants)" → "Mandatory targets" — newtype/derivation
//! validators): the assertions below kill boundary mutations (`<`↔`<=`,
//! round-up↔round-down, floor removal, off-by-one, saturation removal).

use overdrive_core::vm::config::vcpus_for;
use proptest::prelude::*;

proptest! {
    /// S-VM-73 property — for ANY `cpu_milli` in `u32` (including 0 and
    /// values not evenly divisible by 1000), the derived count equals
    /// `max(1, round_up(cpu_milli / 1000))` saturated into `NonZeroU8`,
    /// and is NEVER zero.
    ///
    /// The oracle is recomputed INDEPENDENTLY (widened to `u64`), never by
    /// calling `vcpus_for` — so a mutation to the impl's rounding, floor,
    /// or saturation diverges from this oracle and the property fails. The
    /// hand-computed boundary examples below are the second, fully
    /// arithmetic-independent guard.
    #[test]
    fn vcpus_for_is_ceil_floored_at_one_and_never_zero(cpu_milli in any::<u32>()) {
        let derived = vcpus_for(cpu_milli).get();

        // max(1, round_up(cpu_milli / 1000)), saturated into u8 so the
        // value inhabits NonZeroU8 (max 255) — recomputed in the u64
        // domain, independent of the u32-domain impl.
        let wide = u64::from(cpu_milli).div_ceil(1000).max(1).min(u64::from(u8::MAX));
        let expected = u8::try_from(wide).expect("bounded to u8::MAX by the min above");

        prop_assert_eq!(derived, expected);
        prop_assert!(derived >= 1, "a VM can never have zero vCPUs");
    }
}

/// Boundary examples — the strongest mutation killers. Pinned exact
/// values at every interesting point: the floor (0, sub-core, exact
/// multiples, just-below), the round-up boundary (just-above a multiple),
/// and the `u8` saturation ceiling.
#[test]
fn vcpus_for_pins_the_rounding_and_floor_boundaries() {
    // Floor at 1 — zero, sub-core, and exact-one-core all yield 1.
    assert_eq!(vcpus_for(0).get(), 1, "0 cpu_milli floors at 1 (never a 0-vCPU VM)");
    assert_eq!(vcpus_for(1).get(), 1, "1 cpu_milli rounds up to 1, floored at 1");
    assert_eq!(vcpus_for(250).get(), 1, "a sub-core 250 request floors at 1 (S-VM-70)");
    assert_eq!(vcpus_for(999).get(), 1, "just below one core rounds up to 1");
    assert_eq!(vcpus_for(1000).get(), 1, "exactly one core is 1");

    // Round UP — a non-multiple just past a core boundary rounds up.
    // These kill a round-DOWN mutation (1001 -> 1 would survive round-down).
    assert_eq!(vcpus_for(1001).get(), 2, "1001 rounds UP to 2 (kills round-down)");
    assert_eq!(vcpus_for(1999).get(), 2, "1999 rounds up to 2");
    assert_eq!(vcpus_for(2000).get(), 2, "exactly two cores is 2 (S-VM-69)");
    assert_eq!(vcpus_for(2001).get(), 3, "2001 rounds up to 3");

    // u8 saturation — round_up can exceed 255; NonZeroU8 caps at 255.
    assert_eq!(vcpus_for(255_000).get(), 255, "exactly 255 cores is 255");
    assert_eq!(vcpus_for(255_001).get(), 255, "just past 255 cores saturates at u8::MAX");
    assert_eq!(vcpus_for(u32::MAX).get(), 255, "the largest u32 saturates at u8::MAX");
}
