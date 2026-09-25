//! The VMM launch seccomp program (D-295-R22, ADR-0143).
//!
//! The pure builder is the functional core: [`VmmLaunchSeccompFilter::for_target`]
//! builds the compile target's classic-BPF deny-list program in the parent,
//! with no syscall and no I/O. The imperative shell is the audited launch hook
//! `register_launch_child_hook` in the parent module, which installs the
//! program in the forked child before its first exec.
//!
//! The program exists for `x86_64` only (user ruling 10); every other target is
//! refused at run time with [`LaunchSeccompUnsupportedArch`] (GH #302).

/// The VMM launch seccomp program (D-295-R22, ADR-0143): built in the parent
/// for the compile target's syscall ABI, installed in the child by
/// `register_launch_child_hook`.
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-02")]
pub(super) struct VmmLaunchSeccompFilter {
    program: Vec<libc::sock_filter>,
}

/// No launch seccomp program exists for the compile target.
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-02")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LaunchSeccompUnsupportedArch {
    /// `std::env::consts::ARCH`.
    pub(super) target_arch: &'static str,
}

#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-02")]
impl VmmLaunchSeccompFilter {
    /// Build the program for the compile target. Pure: no syscall and no I/O.
    /// Guarantees `program().len() <= usize::from(u16::MAX)`.
    #[expect(clippy::todo, reason = "RED scaffold — DELIVER step 05-02")]
    pub(super) fn for_target() -> Result<Self, LaunchSeccompUnsupportedArch> {
        todo!("RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02")
    }

    /// The instructions, for the child hook and for pure evaluation tests.
    pub(super) fn program(&self) -> &[libc::sock_filter] {
        &self.program
    }
}

/// The deny-list in table order: `(name, libc::<NAME> as u32)`.
///
/// Each value is the build target's `libc` constant taken as its low 32 bits,
/// which is the kernel's `unsigned int cmd` (`fs/ioctl.c:583`), so upper bits
/// a caller sets cannot evade the match.
#[allow(dead_code, reason = "RED scaffold: consumed in DELIVER step 05-02")]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the kernel matches the ioctl request as its low 32 bits (fs/ioctl.c:583)"
)]
pub(super) const VMM_LAUNCH_DENIED_IOCTLS: [(&str, u32); 13] = [
    ("SIOCSIFHWADDR", libc::SIOCSIFHWADDR as u32),
    ("TUNSETOWNER", libc::TUNSETOWNER as u32),
    ("TUNSETGROUP", libc::TUNSETGROUP as u32),
    ("TUNSETPERSIST", libc::TUNSETPERSIST as u32),
    ("TUNSETCARRIER", libc::TUNSETCARRIER as u32),
    ("TUNSETDEBUG", libc::TUNSETDEBUG as u32),
    ("TUNSETLINK", libc::TUNSETLINK as u32),
    ("TUNSETTXFILTER", libc::TUNSETTXFILTER as u32),
    ("TUNATTACHFILTER", libc::TUNATTACHFILTER as u32),
    ("TUNDETACHFILTER", libc::TUNDETACHFILTER as u32),
    ("TUNSETSTEERINGEBPF", libc::TUNSETSTEERINGEBPF as u32),
    ("TUNSETFILTEREBPF", libc::TUNSETFILTEREBPF as u32),
    ("TUNSETQUEUE", libc::TUNSETQUEUE as u32),
];
