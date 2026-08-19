//! PROBE increment-c — Landlock denial evidence for P5 / US-VM-7 AC 1(b).
//!
//! Why this binary exists at all: `cloud-hypervisor` exposes **no command that
//! opens an arbitrary path**, so the VMM itself cannot be asked to demonstrate
//! "denied outside the ruleset". A sibling process is not covered by the VMM's
//! ruleset either. The only honest way to capture the denial is for a process
//! we control to install the *same* ruleset over the *same* path set the
//! confined VMM was given, and then attempt the opens.
//!
//! What this proves: that the path set handed to `--landlock` /
//! `--landlock-rules` denies everything outside it, with a concrete errno.
//! What it does NOT prove: that CH's internal ruleset is byte-identical to this
//! one (CH also auto-derives rules from its own VM config). The run script
//! passes exactly the paths CH was given, and prints them, so the reader can
//! judge the correspondence rather than take it on trust.
//!
//! Usage:
//!   landlock-denial ro:<path> rw:<path> ... -- allow:<path> deny:<path> ...
//!
//! `allow:` targets are expected to open; `deny:` targets are expected to fail.
//! Exit code 0 iff every expectation held.

use std::ffi::CString;

// asm-generic syscall table (aarch64 and x86_64 agree on these three).
const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;

const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1 << 0;
const LANDLOCK_RULE_PATH_BENEATH: libc::c_int = 1;

// include/uapi/linux/landlock.h — LANDLOCK_ACCESS_FS_*
const A_EXECUTE: u64 = 1 << 0;
const A_WRITE_FILE: u64 = 1 << 1;
const A_READ_FILE: u64 = 1 << 2;
const A_READ_DIR: u64 = 1 << 3;
const A_REMOVE_DIR: u64 = 1 << 4;
const A_REMOVE_FILE: u64 = 1 << 5;
const A_MAKE_CHAR: u64 = 1 << 6;
const A_MAKE_DIR: u64 = 1 << 7;
const A_MAKE_REG: u64 = 1 << 8;
const A_MAKE_SOCK: u64 = 1 << 9;
const A_MAKE_FIFO: u64 = 1 << 10;
const A_MAKE_BLOCK: u64 = 1 << 11;
const A_MAKE_SYM: u64 = 1 << 12;
const A_REFER: u64 = 1 << 13; // ABI 2
const A_TRUNCATE: u64 = 1 << 14; // ABI 3
const A_IOCTL_DEV: u64 = 1 << 15; // ABI 5

/// `struct landlock_ruleset_attr` — only the ABI-1 minimum field is passed, so
/// the same code works on every ABI >= 1.
#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

/// `struct landlock_path_beneath_attr` — **packed** in the UAPI header (12 bytes).
#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

fn abi_version() -> i64 {
    unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    }
}

/// The full fs-access mask the kernel at this ABI can handle. Handling every
/// bit is the strict posture: anything not explicitly granted on a path is
/// denied there.
fn handled_mask(abi: i64) -> u64 {
    let mut m = A_EXECUTE
        | A_WRITE_FILE
        | A_READ_FILE
        | A_READ_DIR
        | A_REMOVE_DIR
        | A_REMOVE_FILE
        | A_MAKE_CHAR
        | A_MAKE_DIR
        | A_MAKE_REG
        | A_MAKE_SOCK
        | A_MAKE_FIFO
        | A_MAKE_BLOCK
        | A_MAKE_SYM;
    if abi >= 2 {
        m |= A_REFER;
    }
    if abi >= 3 {
        m |= A_TRUNCATE;
    }
    if abi >= 5 {
        m |= A_IOCTL_DEV;
    }
    m
}

fn read_access(abi: i64) -> u64 {
    let mut a = A_EXECUTE | A_READ_FILE | A_READ_DIR;
    if abi >= 5 {
        a |= A_IOCTL_DEV;
    }
    a
}

fn write_access(abi: i64) -> u64 {
    read_access(abi)
        | A_WRITE_FILE
        | A_REMOVE_DIR
        | A_REMOVE_FILE
        | A_MAKE_CHAR
        | A_MAKE_DIR
        | A_MAKE_REG
        | A_MAKE_SOCK
        | A_MAKE_FIFO
        | A_MAKE_BLOCK
        | A_MAKE_SYM
        | if abi >= 2 { A_REFER } else { 0 }
        | if abi >= 3 { A_TRUNCATE } else { 0 }
}

/// Access rights the kernel only accepts on a DIRECTORY `parent_fd`. Passing
/// any of these with a regular-file or device `parent_fd` makes
/// `landlock_add_rule` return EINVAL — which is what the first run of this
/// probe hit on the kernel `Image` and on `/dev/kvm`.
const DIR_ONLY: u64 = A_READ_DIR
    | A_REMOVE_DIR
    | A_REMOVE_FILE
    | A_MAKE_CHAR
    | A_MAKE_DIR
    | A_MAKE_REG
    | A_MAKE_SOCK
    | A_MAKE_FIFO
    | A_MAKE_BLOCK
    | A_MAKE_SYM
    | A_REFER;

fn is_dir(path: &str) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

fn open_path_fd(path: &str) -> Result<libc::c_int, std::io::Error> {
    let c = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

/// Attempt a real open and report the raw errno. Read-only is enough: Landlock
/// denies the open itself, so O_RDONLY is the weakest request that can be
/// refused — a denial here is not an artifact of asking for write.
fn try_open(path: &str) -> (bool, i32, String) {
    let c = CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd >= 0 {
        unsafe { libc::close(fd) };
        return (true, 0, "OK".into());
    }
    let e = std::io::Error::last_os_error();
    let errno = e.raw_os_error().unwrap_or(-1);
    (false, errno, format!("{e}"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let rules = &args[..split];
    let targets = if split < args.len() { &args[split + 1..] } else { &[][..] };

    let abi = abi_version();
    println!("=== landlock-denial probe (pid={})", std::process::id());
    println!("--- landlock ABI version reported by kernel: {abi}");
    if abi < 1 {
        println!(
            "!!! Landlock unavailable on this kernel (abi={abi}); cannot produce denial evidence"
        );
        std::process::exit(2);
    }

    let handled = handled_mask(abi);
    println!("--- handled_access_fs mask: 0x{handled:x}");
    println!(
        "--- read access:  0x{:x}    write access: 0x{:x}",
        read_access(abi),
        write_access(abi)
    );

    let attr = RulesetAttr { handled_access_fs: handled };
    let ruleset_fd = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const RulesetAttr,
            std::mem::size_of::<RulesetAttr>(),
            0u32,
        )
    };
    if ruleset_fd < 0 {
        println!("!!! landlock_create_ruleset failed: {}", std::io::Error::last_os_error());
        std::process::exit(2);
    }
    println!("--- landlock_create_ruleset -> fd {ruleset_fd}");

    println!();
    println!("--- ruleset path set (identical to the paths the confined VMM was given):");
    for r in rules {
        let (mode, path) = match r.split_once(':') {
            Some((m, p)) => (m, p),
            None => {
                println!("!!! malformed rule spec {r:?} (want ro:<path> / rw:<path>)");
                std::process::exit(2);
            }
        };
        let mut access = match mode {
            "ro" => read_access(abi),
            "rw" => write_access(abi),
            _ => {
                println!("!!! unknown rule mode {mode:?}");
                std::process::exit(2);
            }
        };
        let dir = is_dir(path);
        if !dir {
            access &= !DIR_ONLY;
        }
        let parent_fd = match open_path_fd(path) {
            Ok(fd) => fd,
            Err(e) => {
                println!("    [{mode}] {path}  -> SKIPPED (cannot O_PATH: {e})");
                continue;
            }
        };
        let pb = PathBeneathAttr { allowed_access: access, parent_fd };
        let rc = unsafe {
            libc::syscall(
                SYS_LANDLOCK_ADD_RULE,
                ruleset_fd,
                LANDLOCK_RULE_PATH_BENEATH,
                &pb as *const PathBeneathAttr,
                0u32,
            )
        };
        if rc != 0 {
            println!("!!! landlock_add_rule({path}) failed: {}", std::io::Error::last_os_error());
            std::process::exit(2);
        }
        let kind = if dir { "dir " } else { "file" };
        println!("    [{mode}] {kind} {path}  -> rule added (access=0x{access:x})");
        unsafe { libc::close(parent_fd) };
    }

    // Population diff (.claude/rules/debugging.md § 5): the SAME opens, by the
    // SAME process, before the ruleset is enforced. Without this control an
    // EACCES after restrict_self is indistinguishable from a path that was
    // never openable in the first place.
    println!();
    println!("=== open() attempts BEFORE landlock_restrict_self (control population)");
    for t in targets {
        let path = t.split_once(':').map(|(_, p)| p).unwrap_or(t.as_str());
        let (ok, errno, msg) = try_open(path);
        let outcome =
            if ok { "OPENED".to_string() } else { format!("failed errno={errno} ({msg})") };
        println!("    {path}\n        -> {outcome}");
    }

    // Landlock requires no_new_privs (or CAP_SYS_ADMIN); CH sets this too.
    let nnp = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    println!();
    println!("--- prctl(PR_SET_NO_NEW_PRIVS, 1) -> {nnp}");

    let rc = unsafe { libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, ruleset_fd, 0u32) };
    if rc != 0 {
        println!("!!! landlock_restrict_self failed: {}", std::io::Error::last_os_error());
        std::process::exit(2);
    }
    println!("--- landlock_restrict_self -> 0   *** RULESET IS NOW ENFORCED ON THIS PROCESS ***");
    unsafe { libc::close(ruleset_fd as libc::c_int) };

    println!();
    println!("=== open() attempts AFTER landlock_restrict_self");
    let mut failures = 0usize;
    for t in targets {
        let (expect, path) = match t.split_once(':') {
            Some((e, p)) => (e, p),
            None => {
                println!("!!! malformed target spec {t:?}");
                std::process::exit(2);
            }
        };
        let (ok, errno, msg) = try_open(path);
        let expected_ok = expect == "allow";
        let verdict = if ok == expected_ok { "as expected" } else { "UNEXPECTED" };
        if ok != expected_ok {
            failures += 1;
        }
        let outcome =
            if ok { "OPENED".to_string() } else { format!("DENIED errno={errno} ({msg})") };
        println!("    expect={expect:<5} {path}");
        println!("        -> {outcome}   [{verdict}]");
    }

    println!();
    if failures == 0 {
        println!("[DENIAL PROBE] VERDICT: every allow: opened and every deny: was refused");
        std::process::exit(0);
    }
    println!("[DENIAL PROBE] VERDICT: FAILED — {failures} expectation(s) violated");
    std::process::exit(1);
}
