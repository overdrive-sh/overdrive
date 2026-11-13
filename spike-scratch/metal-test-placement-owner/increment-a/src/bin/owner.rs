//! The probe owner: optionally init of its own PID namespace, anchored to a
//! pidfd opened in the HOST namespace before the unshare, serving a unix
//! socket. With `--vm` it boots one guest through the production guest kit.
//!
//! args: <dir> <key> <anchor-host-pid> [--no-pidns] [--vm] [--cid=N] [--startup-ms=N]
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ownerprobe::{descendants, first_nspid, ns_link, now_ms, pidfd_open, readable, self_nspid};
use overdrive_testing::guest_kit::{GuestFile, GuestImageSpec, GuestKit, ScriptedBootExtra, WorkDir};
use overdrive_testing::vm_fixture::default_staging_root;

fn log(msg: impl AsRef<str>) {
    eprintln!("OWNER t={} {}", now_ms(), msg.as_ref());
}

fn tool(name: &str) -> GuestFile {
    let host = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
        .into_iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("tool {name} missing"));
    GuestFile { host, guest: Path::new("/usr/bin").join(name), with_shared_libraries: true }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&a[1]);
    let key = a[2].clone();
    let anchor: u32 = a[3].parse().expect("anchor pid");
    let flag = |f: &str| a[4..].iter().any(|x| x == f);
    let value = |p: &str| a[4..].iter().find_map(|x| x.strip_prefix(p)).map(str::to_owned);
    let pidns = !flag("--no-pidns");
    let vm = flag("--vm");
    let cid: u32 = value("--cid=").and_then(|v| v.parse().ok()).unwrap_or(0);
    let startup_ms: u64 = value("--startup-ms=").and_then(|v| v.parse().ok()).unwrap_or(0);
    log(format!("start argv_anchor={anchor} pidns={pidns} vm={vm} cid={cid} startup_ms={startup_ms}"));

    // The anchor pidfd is opened HERE, in the host PID namespace, before the
    // unshare: `pidfd_open(pid)` reads `pid` in the caller's namespace.
    let anchor_fd = match pidfd_open(anchor) {
        Ok(fd) => fd,
        Err(e) => {
            log(format!("REFUSE: pidfd_open(anchor={anchor}) failed errno={e}"));
            std::process::exit(2);
        }
    };
    log(format!("anchor pidfd open in host pidns ({})", ns_link("pid")));

    if pidns {
        // SAFETY: unshare/fork before any thread exists; the child continues.
        unsafe {
            if libc::unshare(libc::CLONE_NEWPID) != 0 {
                log(format!("REFUSE: unshare(CLONE_NEWPID) failed errno={}", ownerprobe::errno()));
                std::process::exit(3);
            }
            match libc::fork() {
                -1 => std::process::exit(4),
                0 => {}
                child => {
                    log(format!("outer forked owner proper, host_pid={child}; outer exits"));
                    libc::_exit(0);
                }
            }
        }
    }

    let host_pid = first_nspid(&self_nspid());
    log(format!(
        "owner proper getpid={} host_pid={host_pid} nspid={} pidns={} netns={}",
        std::process::id(),
        self_nspid(),
        ns_link("pid"),
        ns_link("net")
    ));
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join(format!("{key}.owners"))) {
        let _ = writeln!(f, "start t={} getpid={} host_pid={host_pid}", now_ms(), std::process::id());
    }
    match pidfd_open(anchor) {
        Ok(_) => log("late pidfd_open(anchor) from inside the owner: OK (anchor visible)"),
        Err(e) => log(format!("late pidfd_open(anchor) from inside the owner: errno={e} (anchor NOT visible by host pid)")),
    }
    log(format!("inherited anchor pidfd readable now: {}", readable(anchor_fd.as_raw_fd(), 0)));

    std::thread::sleep(Duration::from_millis(startup_ms));

    let mut guest = None;
    if vm {
        let begun = Instant::now();
        let staging = default_staging_root();
        let kit = GuestKit::provision(&staging).expect("provision");
        let work = WorkDir::create(&staging, "ownerprobe").expect("workdir");
        let mut spec = GuestImageSpec::scripted();
        spec.extra = vec![tool("env"), tool("sleep")];
        let image = kit.image(work.path(), "ownerprobe", &spec).expect("image");
        let staged = begun.elapsed();
        let g = kit
            .boot_scripted_with(
                &image,
                cid,
                &work.path().join("run"),
                &ScriptedBootExtra { cmdline: Vec::new(), memory_mib: Some(512), vcpus: Some(2) },
            )
            .expect("boot");
        log(format!("guest booted staging_ms={} boot_ms={}", staged.as_millis(), (begun.elapsed() - staged).as_millis()));
        guest = Some((g, work));
    }

    // Members: every process below this one, with its NSpid chain.
    let members: Vec<String> = descendants(host_pid)
        .iter()
        .map(|p| format!("{}\t{}\t{}\t{}\t{}", p.pid, p.nspid, p.comm, p.ppid, p.state))
        .collect();
    let _ = fs::write(dir.join(format!("{key}.members")), members.join("\n") + "\n");
    log(format!("members={}", members.len()));

    let sock = dir.join(format!("{key}.sock"));
    let listener = UnixListener::bind(&sock).expect("bind");
    log(format!("serving {} (path len {})", sock.display(), sock.as_os_str().len()));
    let start_ms = now_ms();
    loop {
        let mut fds = [
            libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: anchor_fd.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: two valid pollfds.
        unsafe { libc::poll(fds.as_mut_ptr(), 2, 200) };
        if fds[1].revents & libc::POLLIN != 0 {
            log("ANCHOR FIRED: exiting WITHOUT killing the guest (the PID namespace must)");
            let _ = fs::remove_file(&sock);
            std::process::exit(0);
        }
        if fds[0].revents & libc::POLLIN == 0 {
            continue;
        }
        let Ok((stream, _)) = listener.accept() else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let mut out = &stream;
        match line.trim_end() {
            "HELLO" => {
                let _ = writeln!(
                    out,
                    "OWNER\t{}\t{host_pid}\t{}\t{}\t{start_ms}",
                    std::process::id(),
                    ns_link("net"),
                    ns_link("pid")
                );
            }
            "EXEC" => {
                let code = match guest.as_mut() {
                    Some((g, _)) => g
                        .peer()
                        .exec(&["/usr/bin/env", "OVD_OWNER_PROBE=1"], Duration::from_secs(60))
                        .map_or_else(|e| format!("err:{e}"), |c| c.to_string()),
                    None => "novm".to_owned(),
                };
                let _ = writeln!(out, "EXIT\t{code}");
            }
            "EXECSLEEP" => {
                // a long in-guest exec: the hard case for an owner crash
                let code = match guest.as_mut() {
                    Some((g, _)) => g
                        .peer()
                        .exec(&["/usr/bin/sleep", "40"], Duration::from_secs(60))
                        .map_or_else(|e| format!("err:{e}"), |c| c.to_string()),
                    None => "novm".to_owned(),
                };
                let _ = writeln!(out, "EXIT\t{code}");
            }
            "QUIT" => {
                let _ = writeln!(out, "BYE");
                log("QUIT: exiting WITHOUT killing the guest (the PID namespace must)");
                let _ = fs::remove_file(&sock);
                std::process::exit(0);
            }
            other => {
                let _ = writeln!(out, "ERR\t{other}");
            }
        }
    }
}
