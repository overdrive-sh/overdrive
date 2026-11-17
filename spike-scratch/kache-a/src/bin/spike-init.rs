//! PID 1 of the spike's initramfs. Loads virtiofs, mounts the host root
//! (shared read-only over virtiofs, tag `rootfs`) and hands over to the shell
//! script named by `SPIKE_INIT`, which the kernel passes as an environment
//! variable from the command line. Built static for x86_64-unknown-linux-musl.
use std::ffi::CString;
use std::ptr;

fn c(s: &str) -> CString {
    CString::new(s).expect("no NUL")
}

fn die(what: &str) -> ! {
    let err = std::io::Error::last_os_error();
    eprintln!("spike-init: {what}: {err}");
    unsafe {
        libc::sync();
        libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
    }
    loop {
        std::thread::park();
    }
}

fn mount(src: &str, dst: &str, fs: &str, flags: libc::c_ulong) {
    let (s, d, f) = (c(src), c(dst), c(fs));
    // SAFETY: valid NUL-terminated strings; data is null.
    if unsafe { libc::mount(s.as_ptr(), d.as_ptr(), f.as_ptr(), flags, ptr::null()) } != 0 {
        die(&format!("mount {src} on {dst} ({fs})"));
    }
}

fn main() {
    mount("devtmpfs", "/dev", "devtmpfs", 0);
    mount("proc", "/proc", "proc", 0);
    let module = std::fs::File::open("/virtiofs.ko").unwrap_or_else(|_| die("open virtiofs.ko"));
    let params = c("");
    use std::os::fd::AsRawFd;
    // SAFETY: valid fd and NUL-terminated params.
    if unsafe { libc::syscall(libc::SYS_finit_module, module.as_raw_fd(), params.as_ptr(), 0) } != 0 {
        die("finit_module virtiofs");
    }
    mount("rootfs", "/newroot", "virtiofs", libc::MS_RDONLY);
    mount("/dev", "/newroot/dev", "", libc::MS_MOVE);
    let proc_ = c("/proc");
    // SAFETY: valid path.
    unsafe { libc::umount2(proc_.as_ptr(), libc::MNT_DETACH) };
    let newroot = c("/newroot");
    let dot = c(".");
    let slash = c("/");
    // SAFETY: valid paths; standard switch_root sequence.
    unsafe {
        if libc::chdir(newroot.as_ptr()) != 0 {
            die("chdir /newroot");
        }
        if libc::mount(dot.as_ptr(), slash.as_ptr(), ptr::null(), libc::MS_MOVE, ptr::null()) != 0 {
            die("move root");
        }
        if libc::chroot(dot.as_ptr()) != 0 {
            die("chroot");
        }
        libc::chdir(slash.as_ptr());
    }
    let script = std::env::var("SPIKE_INIT").unwrap_or_else(|_| die("SPIKE_INIT unset"));
    let bash = c("/bin/bash");
    let argv = [c("bash"), c(&script)];
    let argv_ptrs: Vec<*const libc::c_char> =
        argv.iter().map(|a| a.as_ptr()).chain(std::iter::once(ptr::null())).collect();
    let env: Vec<CString> = std::env::vars().map(|(k, v)| c(&format!("{k}={v}"))).collect();
    let env_ptrs: Vec<*const libc::c_char> =
        env.iter().map(|a| a.as_ptr()).chain(std::iter::once(ptr::null())).collect();
    // SAFETY: NULL-terminated argv/envp of valid C strings.
    unsafe { libc::execve(bash.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr()) };
    die("execve /bin/bash");
}
