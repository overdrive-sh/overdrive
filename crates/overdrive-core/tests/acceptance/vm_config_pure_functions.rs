//! Acceptance scenarios for the `VmConfig` anti-corruption pure functions —
//! `microvm-driver-cloud-hypervisor` (GH #42), Slice 01, step 01-01.
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § Slice 01 (S-VM-08, S-VM-16, S-VM-17, S-VM-18, S-VM-20) and ADR-0082
//! §§ D2.1-D2.5. Each of these is a `@property` scenario and a **mandatory
//! mutation target** per the DESIGN handoff to DELIVER.
//!
//! **Step 01-01 activates THREE of the five scaffolds** — S-VM-08
//! (`VmConfinement::seccomp_arg`), S-VM-16 (`DiskAttachment::to_disk_arg`),
//! S-VM-17 (`KernelImage::validate`) — against
//! `overdrive_core::vm::config`. S-VM-18 (`MemoryPlan::derive`'s
//! `guest_bytes == cgroup_max_bytes` unrepresentability) and S-VM-20
//! (`reserve_bytes` itself) stay `#[should_panic(expected = "RED
//! scaffold")]`: both ultimately call `reserve_bytes`, which ships as a
//! `todo!()` this step (ADR-0082 §D2.3 — a measured DELIVER dependency,
//! never a guess) and lands GREEN in step 01-05.

#![allow(clippy::missing_panics_doc)]

use std::path::PathBuf;

use overdrive_core::vm::config::{
    DiskAttachment, Gid, HostArch, KERNEL_MAGIC_WINDOW, KernelImage, VmConfinement, VmmIdentity,
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

/// S-VM-18 — `MemoryPlan::derive(declared)` is the ONLY constructor;
/// `guest_bytes() == cgroup_max_bytes()` is not representable (C-3 / SD-4).
///
/// STAYS RED this step: `cgroup_max_bytes()` is derived through
/// `reserve_bytes(guest_bytes)`, which ships as a `todo!()` until step
/// 01-05 measures the real figure (ADR-0082 §D2.3 Consequences — a hard
/// DELIVER dependency, not a guess). Activating this proptest today would
/// panic on every case, not fail-for-the-right-reason.
#[test]
#[should_panic(expected = "RED scaffold")]
fn memory_plan_derive_makes_guest_bytes_equal_cgroup_max_unrepresentable() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-18 / for any declared u64, \
         MemoryPlan::derive(declared).guest_bytes() == declared and \
         cgroup_max_bytes() is STRICTLY GREATER -- ADR-0082 §D2.3, correction \
         C-3/SD-4, mandatory mutation target)"
    );
}

/// S-VM-20 — `reserve_bytes(guest_bytes)` is a measured constant (hard
/// DELIVER dependency), never a guess, and never persisted.
///
/// STAYS RED this step: `reserve_bytes`'s body is a `todo!("RED scaffold: \
/// ...")` until step 01-05 measures the real bound via `memory.current` /
/// `memory.stat` against a real boot (ADR-0082 §D2.3 Consequences, intake
/// precedent warning #7).
#[test]
#[should_panic(expected = "RED scaffold")]
fn reserve_bytes_is_measured_not_guessed() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-20 / reserve_bytes ships as a \
         hard DELIVER dependency -- measure via memory.current / memory.stat \
         against a real boot, NEVER RSS (host page tables are invisible to RSS); \
         cite the measurement in the function's own docstring before writing a \
         body -- ADR-0082 §D2.3 Consequences, intake precedent warning #7)"
    );
}
