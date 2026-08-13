//! Acceptance scenarios for the `VmConfig` anti-corruption pure functions —
//! `microvm-driver-cloud-hypervisor` (GH #42), Slice 01, steps 01-01 and
//! 01-05.
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § Slice 01 (S-VM-08, S-VM-16, S-VM-17, S-VM-18, S-VM-20) and ADR-0082
//! §§ D2.1-D2.5. Each of these is a `@property` scenario and a **mandatory
//! mutation target** per the DESIGN handoff to DELIVER.
//!
//! **Step 01-01 activated THREE of the five scaffolds** — S-VM-08
//! (`VmConfinement::seccomp_arg`), S-VM-16 (`DiskAttachment::to_disk_arg`),
//! S-VM-17 (`KernelImage::validate`) — against
//! `overdrive_core::vm::config`. **Step 01-05 activates the remaining two**
//! — S-VM-18 (`MemoryPlan::derive`'s `guest_bytes == cgroup_max_bytes`
//! unrepresentability) and S-VM-20 (`reserve_bytes` itself) — now that
//! `reserve_bytes` has a real, measured body (see its own docstring in
//! `overdrive_core::vm::config` for the memory.current / memory.stat
//! measurement table it is derived from).

#![allow(clippy::missing_panics_doc)]

use std::path::PathBuf;

use overdrive_core::vm::config::{
    DiskAttachment, Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelImage, MemoryPlan, VmConfinement,
    VmmIdentity, reserve_bytes,
};
use proptest::prelude::*;

proptest! {
    /// S-VM-08 — `VmConfinement::seccomp_arg()` always renders the literal
    /// `"true"`; no code path can produce `"false"` or `"log"`.
    #[test]
    fn vm_confinement_seccomp_arg_is_never_weakened(
        uid in any::<u32>(),
        gid_raw in any::<u32>(),
        supplementary_raw in proptest::collection::vec(any::<u32>(), 0..6),
        rlimit_nofile in any::<u64>(),
    ) {
        // Given any VmConfinement value the driver could construct.
        let identity = VmmIdentity {
            uid,
            gid: Gid::new(gid_raw),
            supplementary: supplementary_raw.into_iter().map(Gid::new).collect(),
        };
        let confinement = VmConfinement::confined(identity, rlimit_nofile);

        // When seccomp_arg() renders the --seccomp argument.
        let rendered = confinement.seccomp_arg();

        // Then the rendered value is always the literal "true".
        prop_assert_eq!(rendered, "true");
    }

    /// S-VM-16 — `DiskAttachment::to_disk_arg()` always emits
    /// `image_type=raw` unconditionally (C-2).
    #[test]
    fn disk_attachment_to_disk_arg_always_carries_image_type_raw(
        raw_path in "[a-zA-Z0-9/_.-]{1,80}",
        readonly in any::<bool>(),
    ) {
        // Given any DiskAttachment value (any path, either read_only value).
        let path = PathBuf::from(&raw_path);
        let attachment = DiskAttachment::new(path.clone(), readonly);

        // When to_disk_arg() renders the --disk argument.
        let arg = attachment.to_disk_arg();

        // Then the rendered string always contains "image_type=raw" ...
        prop_assert!(arg.contains("image_type=raw"));
        // ... carries the path ... (hoisted into a binding: proptest's
        // single-arg `prop_assert!` stringifies its condition into the
        // auto-generated failure message, and a literal "{}" embedded in
        // the condition's own source text collides with that).
        let expected_path_arg = format!("path={}", path.display());
        prop_assert!(arg.contains(&expected_path_arg));
        // ... and the readonly marker tracks the constructor argument
        // exactly (this is what makes the assertion mutation-sensitive
        // to the `if self.readonly` branch, not just to the literal).
        prop_assert_eq!(arg.contains("readonly=on"), readonly);
    }

    /// S-VM-17 — `KernelImage::validate(path, arch, header)` rejects a
    /// header that does not match the arch's expected magic, before Cloud
    /// Hypervisor ever sees the file (C-7).
    #[test]
    fn kernel_image_validate_rejects_unloadable_headers_before_ch_sees_the_file(
        arch_is_x86 in any::<bool>(),
        filler in any::<u8>(),
    ) {
        let arch = if arch_is_x86 { HostArch::X86_64 } else { HostArch::Aarch64 };

        // Given a byte header that does not match a bzImage magic (x86_64)
        // or a raw PE Image magic (aarch64) for the given HostArch. A
        // header uniformly filled with one non-magic byte can never
        // coincidentally match "HdrS" (starts 0x48), "\x7fELF" (starts
        // 0x7f), or "ARM\x64" (starts 0x41) -- each magic mixes >= 2
        // distinct byte values, so excluding the three first bytes is
        // sufficient for a uniform-fill buffer to miss every one of them.
        prop_assume!(filler != 0x48 && filler != 0x7f && filler != 0x41);
        let header = vec![filler; KERNEL_MAGIC_WINDOW + 16];
        let path = PathBuf::from("/artifacts/kernel-under-test");

        // When KernelImage::validate is called ...
        let rejected = KernelImage::validate(path.clone(), arch, &header);

        // Then it returns a KernelFormatError naming the format, before
        // any hypervisor process is spawned.
        prop_assert!(rejected.is_err());
        if let Err(err) = rejected {
            prop_assert_eq!(err.arch, arch);
        }

        // And a genuinely valid x86_64 bzImage / aarch64 raw Image header
        // validates -- checked deterministically alongside the randomized
        // rejection above (this is the same scenario's second clause).
        let mut bzimage = vec![0u8; KERNEL_MAGIC_WINDOW];
        bzimage[0x202..0x206].copy_from_slice(b"HdrS");
        prop_assert!(KernelImage::validate(path.clone(), HostArch::X86_64, &bzimage).is_ok());

        let mut vmlinux_elf = vec![0u8; KERNEL_MAGIC_WINDOW];
        vmlinux_elf[0..4].copy_from_slice(b"\x7fELF");
        prop_assert!(KernelImage::validate(path.clone(), HostArch::X86_64, &vmlinux_elf).is_ok());

        let mut aarch64_image = vec![0u8; KERNEL_MAGIC_WINDOW];
        aarch64_image[0x38..0x3c].copy_from_slice(b"ARM\x64");
        prop_assert!(
            KernelImage::validate(path.clone(), HostArch::Aarch64, &aarch64_image).is_ok()
        );

        // A distro vmlinuz on aarch64 is UKI-wrapped, not a raw Image --
        // an x86_64 bzImage header must NOT validate on aarch64 either
        // (cross-arch rejection, not just "any garbage" rejection).
        prop_assert!(KernelImage::validate(path, HostArch::Aarch64, &bzimage).is_err());
    }
}

proptest! {
    /// S-VM-18 — `MemoryPlan::derive(declared)` is the ONLY constructor;
    /// `guest_bytes() == cgroup_max_bytes()` is not representable (C-3 /
    /// SD-4). Lands GREEN in step 01-05: `cgroup_max_bytes()` is derived
    /// through `reserve_bytes(guest_bytes)`, which now has a real,
    /// measured body (see its docstring in `overdrive_core::vm::config`).
    ///
    /// `@mandatory:mutation_target` (already carried at DISTILL time,
    /// `test-scenarios.md:503`).
    ///
    /// `declared == u64::MAX` is EXCLUDED via `prop_assume!`, the same
    /// idiom S-VM-17 above already uses for a structurally-impossible
    /// input: `declared.saturating_add(reserve_bytes(declared))` cannot
    /// represent a value greater than `u64::MAX` in a `u64` field, so at
    /// EXACTLY `declared == u64::MAX` the sum saturates back to `declared`
    /// itself and the strict inequality is unsatisfiable -- a ceiling of
    /// the `u64` domain, not a defect in `reserve_bytes` or `derive`. No
    /// real declared guest-RAM figure is anywhere near this boundary
    /// (`u64::MAX` bytes is ~18.4 exabytes). The excluded point is pinned
    /// explicitly by `memory_plan_derive_saturates_at_u64_max` immediately
    /// below, so the boundary behavior is documented, never silently
    /// dropped.
    #[test]
    fn memory_plan_derive_makes_guest_bytes_equal_cgroup_max_unrepresentable(
        declared in any::<u64>(),
    ) {
        prop_assume!(declared != u64::MAX);

        // Given any declared guest RAM figure (short of the u64 ceiling).
        let plan = MemoryPlan::derive(declared);

        // When MemoryPlan::derive builds the plan, guest_bytes() is
        // exactly the declared figure ...
        prop_assert_eq!(plan.guest_bytes(), declared);
        // ... and cgroup_max_bytes() is STRICTLY GREATER -- a cgroup
        // ceiling equal to guest RAM is an OOM by construction (ADR-0082
        // §D2.3, correction C-3/SD-4).
        prop_assert!(
            plan.cgroup_max_bytes() > plan.guest_bytes(),
            "cgroup_max_bytes ({}) must be strictly greater than guest_bytes \
             ({}) -- ADR-0082 §D2.3, correction C-3/SD-4",
            plan.cgroup_max_bytes(),
            plan.guest_bytes(),
        );
    }
}

/// The single point the S-VM-18 proptest above excludes, pinned
/// explicitly: at `declared == u64::MAX`,
/// `declared.saturating_add(reserve_bytes(..))` has nowhere left to go in
/// a `u64` field and saturates back to `declared` itself, so
/// `cgroup_max_bytes() == guest_bytes()` there, not strictly greater. A
/// ceiling of the `u64` domain (no representable value exceeds
/// `u64::MAX`), not a `reserve_bytes` defect -- documented rather than
/// silently excluded, per `.claude/rules/development.md`'s "No
/// aspirational docs" sibling discipline (honest about what the invariant
/// does NOT cover).
#[test]
fn memory_plan_derive_saturates_at_u64_max() {
    let plan = MemoryPlan::derive(u64::MAX);
    assert_eq!(plan.guest_bytes(), u64::MAX);
    assert_eq!(plan.cgroup_max_bytes(), u64::MAX);
}

proptest! {
    /// S-VM-20 — `reserve_bytes(guest_bytes)` is a measured constant (hard
    /// DELIVER dependency), never a guess, and never persisted. Lands
    /// GREEN in step 01-05 -- see `reserve_bytes`'s own docstring in
    /// `overdrive_core::vm::config` for the real Cloud Hypervisor boot
    /// measurement (`memory.current` / `memory.stat`, seven guest sizes)
    /// this policy derives from.
    ///
    /// `@mandatory:mutation_target` -- ADDED this step, per
    /// `test-scenarios.md:563-571`'s own DISTILL-time note: a `todo!()`
    /// body had nothing to mutate, so the tag was deliberately withheld
    /// there; "the step that replaces the `todo!()` with a real
    /// measurement-derived body adds `@mandatory:mutation_target` to this
    /// scenario in the SAME commit and runs `cargo xtask mutants --file`
    /// against it before closing that step" -- this commit is that step.
    ///
    /// Bounds the returned value for any `guest_bytes`: the lower bound
    /// (`>= 0`) holds structurally on `u64` (no runtime check needed --
    /// asserting it would trip rustc's `unused_comparisons` lint on an
    /// always-true unsigned comparison). The upper bound is a fixed floor
    /// plus a fraction of `guest_bytes`; implemented here as EXACT
    /// equality against that floor+fraction formula (the tightest
    /// possible form of ">= 0 and <= upper bound", and the only shape
    /// strong enough to be a real mutation target) -- hardcoded
    /// independently of `reserve_bytes`'s own private policy constants,
    /// so a mutation to either figure (or the operator between them) is
    /// caught by THIS test rather than self-cancelling against an
    /// imported copy of the same numbers.
    #[test]
    fn reserve_bytes_is_measured_not_guessed(guest_bytes in any::<u64>()) {
        // When reserve_bytes computes the reserve for any guest_bytes ...
        let reserve = reserve_bytes(guest_bytes);

        // Then it equals the documented, measured policy: an 8 MiB floor
        // (comfortably above the largest floor-dominated small-guest
        // reading in the measured table, ~3.80 MiB at 256 MiB) plus
        // guest_bytes/400 (~0.25%, comfortably above the largest observed
        // large-guest marginal rate, ~0.19% between the 4096/8192 MiB
        // readings).
        let upper_bound = (8u64 * 1024 * 1024).saturating_add(guest_bytes / 400);
        prop_assert_eq!(
            reserve,
            upper_bound,
            "reserve_bytes({}) = {} must equal the documented policy (8 MiB \
             floor + guest_bytes/400) -- ADR-0082 §D2.3",
            guest_bytes,
            reserve,
        );
    }
}
