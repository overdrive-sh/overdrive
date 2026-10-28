use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_testing::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
use overdrive_testing::guest_kit::{
    GuestFile, GuestImageSpec, GuestInit, GuestKit, LOOPBACK_MODULE_GUEST_PATH, ScriptedBootExtra,
    ScriptedGuest, WorkDir, host_module_file,
};
use overdrive_testing::guest_lane::{CELL_BOUND, IN_GUEST_TESTS_PATH, InGuestTests};
use overdrive_testing::vm_fixture::default_staging_root;

const TEST: &str =
    "integration::reverse_nat_udp_e2e::every_udp_reply_independently_source_rewritten";
const TLS_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/tls.ko";
const PYTHON_STDLIB_ARCHIVE_GUEST_PATH: &str = "/opt/overdrive-testing/python-stdlib.tar";
const BPF_ARTIFACT_GUEST_PATH: &str = "/home/ubuntu/overdrive/target/bpf/overdrive_bpf.o";

const MODULES: &[(&str, &str)] = &[
    ("net/netfilter/nfnetlink", "/lib/overdrive-testing/modules/nfnetlink.ko"),
    ("net/netfilter/nf_tables", "/lib/overdrive-testing/modules/nf_tables.ko"),
    ("drivers/net/veth", "/lib/overdrive-testing/modules/veth.ko"),
    ("net/ipv4/netfilter/nf_defrag_ipv4", "/lib/overdrive-testing/modules/nf_defrag_ipv4.ko"),
    ("net/ipv6/netfilter/nf_defrag_ipv6", "/lib/overdrive-testing/modules/nf_defrag_ipv6.ko"),
    ("net/netfilter/nf_conntrack", "/lib/overdrive-testing/modules/nf_conntrack.ko"),
    ("net/netfilter/nf_nat", "/lib/overdrive-testing/modules/nf_nat.ko"),
    ("net/ipv4/netfilter/nf_reject_ipv4", "/lib/overdrive-testing/modules/nf_reject_ipv4.ko"),
    ("net/ipv6/netfilter/nf_reject_ipv6", "/lib/overdrive-testing/modules/nf_reject_ipv6.ko"),
    ("net/ipv4/netfilter/nf_tproxy_ipv4", "/lib/overdrive-testing/modules/nf_tproxy_ipv4.ko"),
    ("net/ipv6/netfilter/nf_tproxy_ipv6", "/lib/overdrive-testing/modules/nf_tproxy_ipv6.ko"),
    ("net/netfilter/nft_reject", "/lib/overdrive-testing/modules/nft_reject.ko"),
    ("net/ipv4/netfilter/nft_reject_ipv4", "/lib/overdrive-testing/modules/nft_reject_ipv4.ko"),
    ("net/netfilter/nft_tproxy", "/lib/overdrive-testing/modules/nft_tproxy.ko"),
    ("net/netfilter/nft_nat", "/lib/overdrive-testing/modules/nft_nat.ko"),
    ("net/netfilter/nft_chain_nat", "/lib/overdrive-testing/modules/nft_chain_nat.ko"),
    ("net/netfilter/nft_masq", "/lib/overdrive-testing/modules/nft_masq.ko"),
];

const SETUP: &str = r#"
set -eu
/usr/bin/mkdir -p /sys/fs/cgroup /sys/fs/bpf /var /run/netns
/usr/bin/mount -t cgroup2 none /sys/fs/cgroup
/usr/bin/mount -t bpf bpf /sys/fs/bpf
test ! -e /var/run
test ! -L /var/run
/usr/bin/ln -s /run /var/run
/usr/bin/tar -xf /opt/overdrive-testing/python-stdlib.tar -C /
test -L /var/run
test "$(cd /var/run && pwd -P)" = /run
test -d /var/run/netns
test -r /home/ubuntu/overdrive/target/bpf/overdrive_bpf.o
/usr/bin/ethtool --version
bpf_sha256="$(/usr/bin/python3 -c 'import hashlib; print(hashlib.sha256(open("/home/ubuntu/overdrive/target/bpf/overdrive_bpf.o", "rb").read()).hexdigest())')"
echo "UDP-CAPTURE-DIAGNOSTIC preconditions: cgroup2=mounted bpffs=mounted varrun=alias bpf_sha256=$bpf_sha256 tcpdump=present"
"#;

const TCPDUMP_DIAGNOSTIC: &str = r#"
set -eu
echo 'UDP-CAPTURE-DIAGNOSTIC account-files begin'
if test -r /etc/passwd; then /usr/bin/cat /etc/passwd; else echo '/etc/passwd ABSENT'; fi
if test -r /etc/group; then /usr/bin/cat /etc/group; else echo '/etc/group ABSENT'; fi
echo 'UDP-CAPTURE-DIAGNOSTIC account-files end'
/usr/bin/ip netns add ovd-capdiag
/usr/bin/ip link add ovdcd-root type veth peer name ovdcd-peer
/usr/bin/ip link set ovdcd-peer netns ovd-capdiag
/usr/bin/ip netns exec ovd-capdiag /usr/bin/ip link set lo up
/usr/bin/ip netns exec ovd-capdiag /usr/bin/ip link set ovdcd-peer up
/usr/bin/python3 - <<'PY'
import subprocess
command = [
    '/usr/bin/ip', 'netns', 'exec', 'ovd-capdiag', '/usr/bin/tcpdump',
    '-U', '-n', '-i', 'ovdcd-peer', '-w', '/tmp/ovd-capdiag.pcap', 'udp'
]
print('UDP-CAPTURE-DIAGNOSTIC argv=' + repr(command))
process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
try:
    stdout, stderr = process.communicate(timeout=2)
    state = 'exited-before-readiness-window'
except subprocess.TimeoutExpired:
    process.terminate()
    stdout, stderr = process.communicate(timeout=2)
    state = 'remained-live-for-readiness-window'
print(f'UDP-CAPTURE-DIAGNOSTIC state={state} returncode={process.returncode}')
print('UDP-CAPTURE-DIAGNOSTIC stdout=' + repr(stdout))
print('UDP-CAPTURE-DIAGNOSTIC stderr=' + repr(stderr))
PY
/usr/bin/ip netns del ovd-capdiag
/usr/bin/ip link del ovdcd-root 2>/dev/null || true
"#;

const FAILURE_ARTIFACTS: &str = r#"
set -eu
echo 'S05C-DIAGNOSTIC failure-artifacts begin'
/usr/bin/python3 - <<'PY'
from pathlib import Path
import base64
import hashlib
for root in sorted(Path('/tmp').glob('ovd-multi-c-*')):
    print(f'artifact_dir={root}')
    for path in sorted(root.rglob('*')):
        if not path.is_file():
            continue
        data = path.read_bytes()
        print(f'artifact={path} bytes={len(data)} sha256={hashlib.sha256(data).hexdigest()}')
        print(f'artifact_base64={base64.b64encode(data).decode()}')
PY
echo 'S05C-DIAGNOSTIC failure-artifacts end'
"#;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("increment-a has repository-root great-grandparent")
        .to_path_buf()
}

fn tool(name: &str) -> GuestFile {
    let host = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
        .into_iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("required diagnostic tool {name} is not installed"));
    GuestFile { host, guest: Path::new("/usr/bin").join(name), with_shared_libraries: true }
}

fn python_stdlib_archive(dir: &Path) -> GuestFile {
    let output = Command::new("python3")
        .args(["-c", "import sysconfig; print(sysconfig.get_path('stdlib'), end='')"])
        .output()
        .expect("locate Python standard library");
    assert!(output.status.success(), "Python stdlib query failed");
    let stdlib = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let relative = stdlib.strip_prefix("/").expect("Python stdlib path is absolute");
    let archive = dir.join("python-stdlib.tar");
    let output = Command::new("tar")
        .args(["-cf"])
        .arg(&archive)
        .arg("-C")
        .arg("/")
        .arg(relative)
        .output()
        .expect("archive Python standard library");
    assert!(output.status.success(), "Python stdlib archive failed");
    GuestFile {
        host: archive,
        guest: PathBuf::from(PYTHON_STDLIB_ARCHIVE_GUEST_PATH),
        with_shared_libraries: false,
    }
}

fn tcpdump_account_files(dir: &Path) -> [GuestFile; 2] {
    let passwd = dir.join("tcpdump-passwd");
    let group = dir.join("tcpdump-group");
    std::fs::write(
        &passwd,
        "root:x:0:0:root:/root:/bin/sh\ntcpdump:x:985:985:tcpdump:/nonexistent:/usr/sbin/nologin\n",
    )
    .expect("write diagnostic passwd");
    std::fs::write(&group, "root:x:0:\ntcpdump:x:985:\n").expect("write diagnostic group");
    [
        GuestFile {
            host: passwd,
            guest: PathBuf::from("/etc/passwd"),
            with_shared_libraries: false,
        },
        GuestFile { host: group, guest: PathBuf::from("/etc/group"), with_shared_libraries: false },
    ]
}

fn exec_with_modules(
    guest: &mut ScriptedGuest,
    modules: &[String],
    program: &str,
    args: &[&str],
    bound: Duration,
) -> i32 {
    let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES];
    command.extend(modules.iter().map(String::as_str));
    command.extend(["--", program]);
    command.extend_from_slice(args);
    guest.peer().exec(&command, bound).expect("guest diagnostic command")
}

fn main() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("prepared GuestKit provisions on Metal");
    let tests = InGuestTests::prepared("overdrive-dataplane", "integration")
        .expect("prepared dataplane integration binary");
    let work =
        WorkDir::create(&staging, "udp-capture-account-diagnostic").expect("diagnostic workdir");

    let mut extra: Vec<GuestFile> = [
        "bpftool", "cat", "ethtool", "id", "ip", "kill", "ln", "mkdir", "mount", "nc", "nft",
        "python3", "sh", "ss", "strace", "sysctl", "tar", "tcpdump", "true", "uname",
    ]
    .into_iter()
    .map(tool)
    .collect();
    extra.insert(
        0,
        GuestFile {
            host: tests.binary,
            guest: PathBuf::from(IN_GUEST_TESTS_PATH),
            with_shared_libraries: true,
        },
    );
    extra.push(GuestFile {
        host: repository_root().join("target/bpf/overdrive_bpf.o"),
        guest: PathBuf::from(BPF_ARTIFACT_GUEST_PATH),
        with_shared_libraries: false,
    });
    extra.push(python_stdlib_archive(work.path()));
    extra.extend(tcpdump_account_files(work.path()));
    extra.push(
        host_module_file(&kit, work.path(), "net/tls/tls", Path::new(TLS_MODULE_GUEST_PATH))
            .expect("stage TLS module"),
    );
    let mut modules = vec![LOOPBACK_MODULE_GUEST_PATH.to_owned(), TLS_MODULE_GUEST_PATH.to_owned()];
    for &(relative, guest_path) in MODULES {
        extra.push(
            host_module_file(&kit, work.path(), relative, Path::new(guest_path))
                .unwrap_or_else(|source| panic!("stage {relative}: {source}")),
        );
        modules.push(guest_path.to_owned());
    }

    let image = kit
        .image(
            work.path(),
            "udp-capture-account-diagnostic",
            &GuestImageSpec {
                init: GuestInit::ScriptedPeer,
                forwarder_module: true,
                loopback_module: true,
                extra,
            },
        )
        .expect("build bounded diagnostic guest image");
    let mut guest = kit
        .boot_scripted_with(
            &image,
            FWD_GUEST_BOUNDARY.raw(13),
            &work.path().join("run"),
            &ScriptedBootExtra { cmdline: Vec::new(), memory_mib: Some(4_096), vcpus: Some(2) },
        )
        .expect("boot bounded diagnostic guest");

    assert_eq!(
        exec_with_modules(&mut guest, &modules, "/usr/bin/sh", &["-c", SETUP], CELL_BOUND),
        0,
        "guest preconditions"
    );
    let diagnostic_before = guest.vm.console().len();
    assert_eq!(
        exec_with_modules(
            &mut guest,
            &modules,
            "/usr/bin/sh",
            &["-c", TCPDUMP_DIAGNOSTIC],
            CELL_BOUND,
        ),
        0,
        "tcpdump account diagnostic"
    );
    let diagnostic_console = guest.vm.console();
    println!(
        "===== UDP capture account diagnostic =====\n{}",
        diagnostic_console.get(diagnostic_before..).unwrap_or(&diagnostic_console)
    );
    let console_before = guest.vm.console().len();
    let code = exec_with_modules(
        &mut guest,
        &modules,
        IN_GUEST_TESTS_PATH,
        &["--test-threads=1", "--nocapture", "--exact", TEST],
        CELL_BOUND,
    );
    let console = guest.vm.console();
    let run_console = console.get(console_before..).unwrap_or(&console);
    println!("===== UDP capture bounded exact replay =====\n{run_console}");
    if code != 0 || !run_console.contains("test result: ok. 1 passed; 0 failed;") {
        let _ = exec_with_modules(
            &mut guest,
            &modules,
            "/usr/bin/sh",
            &["-c", FAILURE_ARTIFACTS],
            Duration::from_secs(30),
        );
    }
    let _ = guest.take_peer().power_off();
    let exit = guest.vm.wait_exit(Duration::from_secs(30)).expect("wait guest exit");
    assert!(exit.is_some(), "diagnostic guest powers off");
    assert_eq!(code, 0, "unchanged UDP capture case exits zero");
    assert!(
        run_console.contains("test result: ok. 1 passed; 0 failed;"),
        "unchanged UDP capture case reports one pass"
    );
    assert!(!run_console.contains("[skip]"), "bounded native case cannot skip");
    println!("UDP_CAPTURE_BOUNDED_REPLAY_PASS");
}
