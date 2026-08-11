//! RED scaffolds for the `VmConfig` anti-corruption pure functions —
//! `microvm-driver-cloud-hypervisor` (GH #42), Slice 01.
//!
//! Per `docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md`
//! § Slice 01 (S-VM-08, S-VM-16, S-VM-17, S-VM-18, S-VM-20) and ADR-0082
//! §§ D2.1-D2.5. Each of these is a `@property` scenario and a **mandatory
//! mutation target** per the DESIGN handoff to DELIVER — the design pins
//! the exact signature for each function; DELIVER implements it and
//! converts the corresponding scaffold below into a `proptest!` block
//! (Tier 1, default lane, PBT-full per Mandate 9).
//!
//! Per `.claude/rules/testing.md` § "RED scaffolds": placeholder bodies
//! only. DELIVER fills each function in with the real signature pinned in
//! ADR-0082 — crafters must not improvise a different one
//! (`CLAUDE.md` § "Implement to the design").

#![allow(clippy::missing_panics_doc)]

/// S-VM-08 — `VmConfinement::seccomp_arg()` always renders the literal
/// `"true"`; no code path can produce `"false"` or `"log"`.
#[test]
#[should_panic(expected = "RED scaffold")]
fn vm_confinement_seccomp_arg_is_never_weakened() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-08 / VmConfinement::seccomp_arg \
         renders \"true\" unconditionally -- ADR-0082 §D2.5, mandatory mutation target)"
    );
}

/// S-VM-16 — `DiskAttachment::to_disk_arg()` always emits
/// `image_type=raw` unconditionally (C-2).
#[test]
#[should_panic(expected = "RED scaffold")]
fn disk_attachment_to_disk_arg_always_carries_image_type_raw() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-16 / DiskAttachment::to_disk_arg \
         emits \"image_type=raw\" for every path/read_only combination -- ADR-0082 \
         §D2.1, correction C-2, mandatory mutation target)"
    );
}

/// S-VM-17 — `KernelImage::validate(path, arch, header)` rejects a
/// header that does not match the arch's expected magic, before Cloud
/// Hypervisor ever sees the file (C-7).
#[test]
#[should_panic(expected = "RED scaffold")]
fn kernel_image_validate_rejects_unloadable_headers_before_ch_sees_the_file() {
    panic!(
        "Not yet implemented -- RED scaffold (S-VM-17 / KernelImage::validate is \
         PURE -- caller reads the header bytes, the function does no I/O -- x86_64 \
         accepts bzImage (HdrS at 0x202) or PVH vmlinux; aarch64 accepts raw PE \
         Image (ARM\\x64 at 0x38); a distro vmlinuz on aarch64 correctly fails -- \
         ADR-0082 §D2.4, correction C-7)"
    );
}

/// S-VM-18 — `MemoryPlan::derive(declared)` is the ONLY constructor;
/// `guest_bytes() == cgroup_max_bytes()` is not representable (C-3 / SD-4).
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
