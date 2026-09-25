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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    #![allow(
        clippy::doc_markdown,
        reason = "the repository-mandated CONTRACT_SHAPE declaration is an exact machine-read line"
    )]

    #[cfg(target_arch = "x86_64")]
    use proptest::prelude::*;

    use super::*;

    // Classic-BPF opcode fields (UAPI `linux/bpf_common.h`), restated here so
    // the evaluator is an oracle independent of the builder's constants.
    #[cfg(target_arch = "x86_64")]
    mod cbpf {
        pub(super) const CLASS_MASK: u16 = 0x07;
        pub(super) const LD: u16 = 0x00;
        pub(super) const JMP: u16 = 0x05;
        pub(super) const RET: u16 = 0x06;
        pub(super) const SIZE_W: u16 = 0x00;
        pub(super) const MODE_ABS: u16 = 0x20;
        pub(super) const SRC_K: u16 = 0x00;
        pub(super) const SRC_MASK: u16 = 0x08;
        pub(super) const OP_MASK: u16 = 0xf0;
        pub(super) const JA: u16 = 0x00;
        pub(super) const JEQ: u16 = 0x10;
        pub(super) const JGT: u16 = 0x20;
        pub(super) const JGE: u16 = 0x30;
        pub(super) const JSET: u16 = 0x40;
        pub(super) const RVAL_A: u16 = 0x10;
    }

    // Seccomp actions and the audit/x32 constants (UAPI `linux/seccomp.h`,
    // `linux/audit.h`, `asm/unistd.h`, `linux/elf-em.h`): the test oracle.
    #[cfg(target_arch = "x86_64")]
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    #[cfg(target_arch = "x86_64")]
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    #[cfg(target_arch = "x86_64")]
    const SECCOMP_RET_ERRNO_EPERM: u32 = 0x0005_0000 | 1;
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH_I386: u32 = 0x4000_0003;
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH_AARCH64: u32 = 0xC000_00B7;
    #[cfg(target_arch = "x86_64")]
    const X32_SYSCALL_BIT: u32 = 0x4000_0000;
    /// `__NR_ioctl` on x86_64 (`arch/x86/entry/syscalls/syscall_64.tbl`).
    #[cfg(target_arch = "x86_64")]
    const NR_IOCTL_X86_64: i32 = 16;

    /// The increment-aa x86_64 measurement of the thirteen denied requests
    /// (FD 1009-1023), in table order: the oracle for the `libc` derivation.
    #[cfg(target_arch = "x86_64")]
    const MEASURED_DENIED_IOCTLS: [(&str, u32); 13] = [
        ("SIOCSIFHWADDR", 0x8924),
        ("TUNSETOWNER", 0x4004_54cc),
        ("TUNSETGROUP", 0x4004_54ce),
        ("TUNSETPERSIST", 0x4004_54cb),
        ("TUNSETCARRIER", 0x4004_54e2),
        ("TUNSETDEBUG", 0x4004_54c9),
        ("TUNSETLINK", 0x4004_54cd),
        ("TUNSETTXFILTER", 0x4004_54d1),
        ("TUNATTACHFILTER", 0x4010_54d5),
        ("TUNDETACHFILTER", 0x4010_54d6),
        ("TUNSETSTEERINGEBPF", 0x8004_54e0),
        ("TUNSETFILTEREBPF", 0x8004_54e1),
        ("TUNSETQUEUE", 0x4004_54d9),
    ];

    /// The six requests Cloud Hypervisor v53's `fd=` path issues, and two
    /// read-only requests: every one must be allowed (FD 1025-1028).
    #[cfg(target_arch = "x86_64")]
    const ALLOWED_REQUESTS: [(&str, u64); 8] = [
        ("TUNGETIFF", libc::TUNGETIFF),
        ("TUNSETIFF", libc::TUNSETIFF),
        ("TUNSETVNETHDRSZ", libc::TUNSETVNETHDRSZ),
        ("SIOCGIFMTU", libc::SIOCGIFMTU),
        ("SIOCSIFMTU", libc::SIOCSIFMTU),
        ("TUNSETOFFLOAD", libc::TUNSETOFFLOAD),
        ("TUNGETVNETHDRSZ", libc::TUNGETVNETHDRSZ),
        ("SIOCGIFHWADDR", libc::SIOCGIFHWADDR),
    ];

    /// `struct seccomp_data` (UAPI `linux/seccomp.h`): `nr` at 0, `arch` at 4,
    /// `instruction_pointer` at 8, `args[6]` at 16; 64 bytes, native endian.
    #[cfg(target_arch = "x86_64")]
    fn seccomp_data(nr: i32, arch: u32, instruction_pointer: u64, args: [u64; 6]) -> [u8; 64] {
        let mut data = [0_u8; 64];
        data[0..4].copy_from_slice(&nr.to_ne_bytes());
        data[4..8].copy_from_slice(&arch.to_ne_bytes());
        data[8..16].copy_from_slice(&instruction_pointer.to_ne_bytes());
        for (index, arg) in args.iter().enumerate() {
            let at = 16 + index * 8;
            data[at..at + 8].copy_from_slice(&arg.to_ne_bytes());
        }
        data
    }

    /// A classic-BPF interpreter for the seccomp subset: absolute 32-bit loads
    /// from `seccomp_data`, constant-operand conditional and unconditional
    /// jumps, and returns. Any other instruction, a misaligned or
    /// out-of-range load, or running off the end of the program is an error,
    /// so an evaluation that returns `Ok` is a total, defined verdict.
    #[cfg(target_arch = "x86_64")]
    fn evaluate(program: &[libc::sock_filter], data: &[u8; 64]) -> Result<u32, String> {
        let mut accumulator: u32 = 0;
        let mut pc: usize = 0;
        loop {
            let insn = program.get(pc).ok_or_else(|| format!("fell off the program at {pc}"))?;
            let class = insn.code & cbpf::CLASS_MASK;
            match class {
                cbpf::LD => {
                    if insn.code != cbpf::LD | cbpf::SIZE_W | cbpf::MODE_ABS {
                        return Err(format!("unsupported load {:#06x} at {pc}", insn.code));
                    }
                    let offset = usize::try_from(insn.k).map_err(|e| e.to_string())?;
                    if !offset.is_multiple_of(4) || offset + 4 > data.len() {
                        return Err(format!("invalid seccomp_data load offset {offset} at {pc}"));
                    }
                    accumulator = u32::from_ne_bytes(
                        data[offset..offset + 4].try_into().expect("four bytes"),
                    );
                    pc += 1;
                }
                cbpf::JMP => {
                    if insn.code & cbpf::SRC_MASK != cbpf::SRC_K {
                        return Err(format!("register-operand jump {:#06x} at {pc}", insn.code));
                    }
                    let taken = match insn.code & cbpf::OP_MASK {
                        cbpf::JA => {
                            let skip = usize::try_from(insn.k).map_err(|e| e.to_string())?;
                            pc += 1 + skip;
                            continue;
                        }
                        cbpf::JEQ => accumulator == insn.k,
                        cbpf::JGT => accumulator > insn.k,
                        cbpf::JGE => accumulator >= insn.k,
                        cbpf::JSET => accumulator & insn.k != 0,
                        other => return Err(format!("unsupported jump op {other:#04x} at {pc}")),
                    };
                    pc += 1 + usize::from(if taken { insn.jt } else { insn.jf });
                }
                cbpf::RET => {
                    return if insn.code & cbpf::RVAL_A == cbpf::RVAL_A {
                        Ok(accumulator)
                    } else {
                        Ok(insn.k)
                    };
                }
                other => return Err(format!("unsupported instruction class {other:#04x} at {pc}")),
            }
        }
    }

    /// The accepted verdict model (FD 1030-1052, 1102-1116): foreign audit
    /// architecture → kill; `nr == -1` → allow; any other `nr` with the x32 bit
    /// or above → kill; `ioctl` whose request's low 32 bits are denied → EPERM;
    /// everything else → allow.
    #[cfg(target_arch = "x86_64")]
    fn expected_verdict(nr: i32, arch: u32, request: u64) -> u32 {
        let request_low = u32::try_from(request & u64::from(u32::MAX)).expect("masked to 32 bits");
        if arch != AUDIT_ARCH_X86_64 {
            SECCOMP_RET_KILL_PROCESS
        } else if nr == -1 {
            SECCOMP_RET_ALLOW
        } else if nr.cast_unsigned() >= X32_SYSCALL_BIT {
            SECCOMP_RET_KILL_PROCESS
        } else if nr == NR_IOCTL_X86_64
            && MEASURED_DENIED_IOCTLS.iter().any(|(_, value)| *value == request_low)
        {
            SECCOMP_RET_ERRNO_EPERM
        } else {
            SECCOMP_RET_ALLOW
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn production_program() -> Vec<libc::sock_filter> {
        VmmLaunchSeccompFilter::for_target()
            .expect("the 64-bit x86_64 target has a launch seccomp program")
            .program()
            .to_vec()
    }

    #[cfg(target_arch = "x86_64")]
    fn verdict_of(program: &[libc::sock_filter], nr: i32, arch: u32, request: u64) -> u32 {
        let data = seccomp_data(nr, arch, 0x7f00_dead_beef, [3, request, 0x1000, 0, 0, 0]);
        evaluate(program, &data).unwrap_or_else(|error| {
            panic!(
                "the launch program must return a defined verdict for nr {nr:#x}, arch {arch:#x}, \
                 request {request:#x}: {error}"
            )
        })
    }

    #[cfg(target_arch = "x86_64")]
    fn nr_strategy() -> impl Strategy<Value = i32> {
        prop_oneof![
            4 => any::<i32>(),
            3 => Just(NR_IOCTL_X86_64),
            1 => Just(-1),
            1 => 0_i32..=512,
            1 => 0x4000_0000_i32..=0x4000_0400,
            1 => i32::MIN..=-2,
        ]
    }

    #[cfg(target_arch = "x86_64")]
    fn arch_strategy() -> impl Strategy<Value = u32> {
        prop_oneof![
            6 => Just(AUDIT_ARCH_X86_64),
            1 => Just(AUDIT_ARCH_I386),
            1 => Just(AUDIT_ARCH_AARCH64),
            1 => any::<u32>(),
        ]
    }

    /// `args[1]`: arbitrary, a denied request under arbitrary upper 32 bits,
    /// or an allowed request under arbitrary upper 32 bits.
    #[cfg(target_arch = "x86_64")]
    fn request_strategy() -> impl Strategy<Value = u64> {
        prop_oneof![
            2 => any::<u64>(),
            3 => (0..MEASURED_DENIED_IOCTLS.len(), any::<u32>()).prop_map(|(index, upper)| {
                (u64::from(upper) << 32) | u64::from(MEASURED_DENIED_IOCTLS[index].1)
            }),
            2 => (0..ALLOWED_REQUESTS.len(), any::<u32>())
                .prop_map(|(index, upper)| (u64::from(upper) << 32) | ALLOWED_REQUESTS[index].1),
        ]
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-42 — The launch filter's verdict is total and correct for
    /// every syscall shape.
    /// CONTRACT_SHAPE: pure-function.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-42)"]
    fn the_launch_filter_verdict_partition_is_total() {
        let program = production_program();

        // (a) each denied request, bare and under set upper 32 bits.
        for (name, value) in MEASURED_DENIED_IOCTLS {
            for upper in [0_u64, 0xDEAD_BEEF, u64::from(u32::MAX)] {
                let request = (upper << 32) | u64::from(value);
                assert_eq!(
                    verdict_of(&program, NR_IOCTL_X86_64, AUDIT_ARCH_X86_64, request),
                    SECCOMP_RET_ERRNO_EPERM,
                    "{name} ({request:#x}) must be refused with EPERM whatever its upper bits",
                );
            }
        }

        // (b) the fd= path's requests and read-only requests pass; a
        // non-ioctl syscall carrying a denied value passes.
        for (name, request) in ALLOWED_REQUESTS {
            assert_eq!(
                verdict_of(&program, NR_IOCTL_X86_64, AUDIT_ARCH_X86_64, request),
                SECCOMP_RET_ALLOW,
                "{name} ({request:#x}) must be allowed",
            );
        }
        for nr in [0_i32, 1, 17, 54, 334] {
            for (name, value) in MEASURED_DENIED_IOCTLS {
                assert_eq!(
                    verdict_of(&program, nr, AUDIT_ARCH_X86_64, u64::from(value)),
                    SECCOMP_RET_ALLOW,
                    "syscall {nr} carrying {name} in args[1] is not an ioctl and must be allowed",
                );
            }
        }

        // (c) a foreign audit architecture ends the process, whatever the call.
        for arch in [AUDIT_ARCH_I386, AUDIT_ARCH_AARCH64, 0, AUDIT_ARCH_X86_64 & !0x8000_0000] {
            for nr in [NR_IOCTL_X86_64, 0, -1] {
                assert_eq!(
                    verdict_of(&program, nr, arch, u64::from(MEASURED_DENIED_IOCTLS[1].1)),
                    SECCOMP_RET_KILL_PROCESS,
                    "audit architecture {arch:#x} (nr {nr}) must end the process",
                );
            }
        }

        // (d) x32 and every other nr at or above the x32 bit end the process;
        // the tracer skip nr -1 passes.
        for nr in [
            0x4000_0000 + 514,
            0x4000_0000 + 16,
            0x4000_0000,
            0x4000_0000 + 1,
            i32::MAX,
            -2,
            i32::MIN,
        ] {
            assert_eq!(
                verdict_of(&program, nr, AUDIT_ARCH_X86_64, 0),
                SECCOMP_RET_KILL_PROCESS,
                "nr {nr:#x} (at or above the x32 bit) must end the process",
            );
        }
        for request in [0_u64, u64::from(MEASURED_DENIED_IOCTLS[1].1)] {
            assert_eq!(
                verdict_of(&program, -1, AUDIT_ARCH_X86_64, request),
                SECCOMP_RET_ALLOW,
                "nr -1 (a tracer's syscall skip) must be allowed",
            );
        }

        // Every other syscall record: the verdict is defined and equals the
        // accepted model. A failure prints proptest's minimal input and
        // persists its seed beside this file (`PROPTEST_CASES` is never
        // lowered).
        let config = ProptestConfig { source_file: Some(file!()), ..ProptestConfig::default() };
        let mut runner = proptest::test_runner::TestRunner::new(config);
        let records =
            (nr_strategy(), arch_strategy(), request_strategy(), any::<u64>(), any::<[u64; 5]>());
        let outcome = runner.run(&records, |(nr, arch, request, instruction_pointer, other)| {
            let args = [other[0], request, other[1], other[2], other[3], other[4]];
            let data = seccomp_data(nr, arch, instruction_pointer, args);
            prop_assert_eq!(
                evaluate(&program, &data),
                Ok(expected_verdict(nr, arch, request)),
                "nr {:#x}, arch {:#x}, args[1] {:#x}",
                nr,
                arch,
                request,
            );
            Ok(())
        });
        if let Err(failure) = outcome {
            panic!("the launch program's verdict diverged from the accepted model: {failure}");
        }
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-42 — The launch filter's verdict is total and correct for
    /// every syscall shape (deny-list data, audit constants, exact program).
    /// CONTRACT_SHAPE: pure-function.
    #[cfg(target_arch = "x86_64")]
    #[test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-42)"]
    fn the_deny_list_equals_the_measured_ioctl_numbers_and_the_audit_constants() {
        assert_eq!(
            VMM_LAUNCH_DENIED_IOCTLS, MEASURED_DENIED_IOCTLS,
            "the libc-derived deny-list must equal the increment-aa x86_64 measurement, in table order",
        );
        assert_eq!(
            u32::from(libc::EM_X86_64) | 0x8000_0000 | 0x4000_0000,
            AUDIT_ARCH_X86_64,
            "EM_X86_64 | __AUDIT_ARCH_64BIT | __AUDIT_ARCH_LE must compose to AUDIT_ARCH_X86_64",
        );
        assert_eq!(libc::SYS_ioctl, i64::from(NR_IOCTL_X86_64), "x86_64 ioctl is syscall 16");

        let program = production_program();
        assert!(program.len() <= usize::from(u16::MAX), "sock_fprog.len is a u16");

        let ld_abs = cbpf::LD | cbpf::SIZE_W | cbpf::MODE_ABS;
        let jeq = cbpf::JMP | cbpf::JEQ | cbpf::SRC_K;
        let jge = cbpf::JMP | cbpf::JGE | cbpf::SRC_K;
        let ret = cbpf::RET | cbpf::SRC_K;
        let insn = |code: u16, jt: u8, jf: u8, k: u32| (code, jt, jf, k);
        let mut expected = vec![
            insn(ld_abs, 0, 0, 4),
            insn(jeq, 1, 0, AUDIT_ARCH_X86_64),
            insn(ret, 0, 0, SECCOMP_RET_KILL_PROCESS),
            insn(ld_abs, 0, 0, 0),
            insn(jeq, 17, 0, u32::MAX),
            insn(jge, 0, 1, X32_SYSCALL_BIT),
            insn(ret, 0, 0, SECCOMP_RET_KILL_PROCESS),
            insn(jeq, 0, 14, NR_IOCTL_X86_64.cast_unsigned()),
            insn(ld_abs, 0, 0, 16 + 8),
        ];
        for (row, (_, value)) in MEASURED_DENIED_IOCTLS.iter().enumerate() {
            let to_deny = u8::try_from(13 - row).expect("forward jump fits u8");
            expected.push(insn(jeq, to_deny, 0, *value));
        }
        expected.push(insn(ret, 0, 0, SECCOMP_RET_ALLOW));
        expected.push(insn(ret, 0, 0, SECCOMP_RET_ERRNO_EPERM));

        let actual: Vec<_> = program.iter().map(|i| (i.code, i.jt, i.jf, i.k)).collect();
        assert_eq!(actual.len(), 24, "the launch program is exactly 24 instructions");
        assert_eq!(
            actual, expected,
            "the launch program must be exactly the accepted instruction table (FD 1102-1116)",
        );
        assert_eq!(actual[1].3, 0xC000_003E, "the composed audit value is pinned");
        assert_eq!(actual[5].3, 0x4000_0000, "the x32 syscall bit is pinned");
    }

    /// Outcome anchor: OUT-ND295-BORN-CAPTURED.
    /// S-ND295-44 — A target with no launch filter starts no microVM (the
    /// builder names the unsupported architecture).
    /// CONTRACT_SHAPE: pure-function.
    #[cfg(not(target_arch = "x86_64"))]
    #[test]
    #[ignore = "pending DELIVER step 05-02 (S-ND295-44)"]
    fn a_target_without_a_program_is_unsupported() {
        match VmmLaunchSeccompFilter::for_target() {
            Err(unsupported) => {
                assert_eq!(
                    unsupported,
                    LaunchSeccompUnsupportedArch { target_arch: std::env::consts::ARCH },
                    "the refusal must name the running target architecture",
                );
                #[cfg(target_arch = "aarch64")]
                assert_eq!(unsupported.target_arch, "aarch64");
            }
            Ok(filter) => panic!(
                "no launch seccomp program is accepted for {}, yet the builder produced {} instructions",
                std::env::consts::ARCH,
                filter.program().len(),
            ),
        }
    }
}
