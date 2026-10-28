use std::path::{Path, PathBuf};
use std::time::Duration;

use overdrive_testing::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
use overdrive_testing::guest_kit::{
    DUMMY_MODULE_GUEST_PATH, GuestFile, GuestImageSpec, GuestInit, GuestKit,
    LOOPBACK_MODULE_GUEST_PATH, ScriptedBootExtra, ScriptedGuest, WorkDir, host_module_file,
};
use overdrive_testing::guest_lane::{CELL_BOUND, IN_GUEST_TESTS_PATH, InGuestTests};
use overdrive_testing::vm_fixture::default_staging_root;

const TEST: &str = "integration::dns_responder_bind::probe_refuses_when_no_bindable_port";
const WORKSPACE_FORWARDER_GUEST_PATH: &str =
    "/home/ubuntu/overdrive/target/kmod/7.0.0-38-generic/ovd_fwd.ko";

const MODULES: &[(&str, &str)] = &[
    ("drivers/crypto/ccp/ccp", "/lib/overdrive-testing/modules/ccp.ko"),
    ("virt/lib/irqbypass", "/lib/overdrive-testing/modules/irqbypass.ko"),
    ("arch/x86/kvm/kvm", "/lib/overdrive-testing/modules/kvm.ko"),
    ("arch/x86/kvm/kvm-amd", "/lib/overdrive-testing/modules/kvm-amd.ko"),
    ("drivers/vhost/vhost_iotlb", "/lib/overdrive-testing/modules/vhost_iotlb.ko"),
    ("drivers/vhost/vhost", "/lib/overdrive-testing/modules/vhost.ko"),
    ("drivers/vhost/vhost_vsock", "/lib/overdrive-testing/modules/vhost_vsock.ko"),
    ("net/netfilter/nfnetlink", "/lib/overdrive-testing/modules/nfnetlink.ko"),
    ("net/netfilter/nf_tables", "/lib/overdrive-testing/modules/nf_tables.ko"),
    ("drivers/net/veth", "/lib/overdrive-testing/modules/veth.ko"),
    ("net/tls/tls", "/lib/overdrive-testing/modules/tls.ko"),
    ("net/ipv4/netfilter/nf_defrag_ipv4", "/lib/overdrive-testing/modules/nf_defrag_ipv4.ko"),
    ("net/ipv6/netfilter/nf_defrag_ipv6", "/lib/overdrive-testing/modules/nf_defrag_ipv6.ko"),
    ("net/ipv4/netfilter/nf_reject_ipv4", "/lib/overdrive-testing/modules/nf_reject_ipv4.ko"),
    ("net/ipv6/netfilter/nf_reject_ipv6", "/lib/overdrive-testing/modules/nf_reject_ipv6.ko"),
    ("net/ipv4/netfilter/nf_tproxy_ipv4", "/lib/overdrive-testing/modules/nf_tproxy_ipv4.ko"),
    ("net/ipv6/netfilter/nf_tproxy_ipv6", "/lib/overdrive-testing/modules/nf_tproxy_ipv6.ko"),
    ("net/netfilter/nft_reject", "/lib/overdrive-testing/modules/nft_reject.ko"),
    ("net/ipv4/netfilter/nft_reject_ipv4", "/lib/overdrive-testing/modules/nft_reject_ipv4.ko"),
    ("net/netfilter/nft_tproxy", "/lib/overdrive-testing/modules/nft_tproxy.ko"),
];

const SETUP: &str = r#"
set -eu
/usr/bin/chmod 0660 /dev/kvm /dev/vhost-vsock
/usr/bin/mkdir -p /sys/fs/cgroup /sys/fs/bpf /var /run/netns
/usr/bin/mount -t cgroup2 none /sys/fs/cgroup
/usr/bin/mount -t bpf bpf /sys/fs/bpf
printf '%s\n' '127.0.0.1 localhost' '::1 localhost' > /etc/hosts
printf '%s\n' 'overdrive-cgroup-test' > /proc/sys/kernel/hostname
printf '%s\n' '+cpu +memory +io +pids' > /sys/fs/cgroup/cgroup.subtree_control
/usr/bin/mkdir -p /sys/fs/cgroup/overdrive.slice
printf '%s\n' '+cpu +memory +io +pids' > /sys/fs/cgroup/overdrive.slice/cgroup.subtree_control
test ! -e /var/run
test ! -L /var/run
/usr/bin/ln -s /run /var/run
test -L /var/run
test "$(cd /var/run && pwd -P)" = /run
test -d /var/run/netns
test -f /etc/resolv.conf
test ! -L /etc/resolv.conf
test ! -s /etc/resolv.conf
/usr/bin/setsid /bin/sleep 0
/usr/bin/ip -4 route add local 100.95.0.1/32 dev lo proto 247
echo 'DNS53-NEGATIVE-OLD diagnostic: gateway_route=present'
"#;

const CGROUP_EXEC: &str = r#"
set -eu
ulimit -Hn 376832
ulimit -Sn 376832
exec "$@"
"#;

fn tool(name: &str) -> GuestFile {
    let host = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
        .into_iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("required diagnostic tool {name} is not installed"));
    GuestFile { host, guest: Path::new("/usr/bin").join(name), with_shared_libraries: true }
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
    assert_eq!(
        kit.kernel_release, "7.0.0-38-generic",
        "bounded diagnostic pins the qualified kernel release"
    );
    let tests = InGuestTests::prepared("overdrive-control-plane", "integration")
        .expect("prepared control-plane integration binary");
    let work = WorkDir::create(&staging, "control-plane-native-closure-diagnostic")
        .expect("diagnostic workdir");

    let mut extra: Vec<GuestFile> = [
        "cat", "chmod", "ethtool", "ip", "ln", "mkdir", "mount", "nft", "setsid", "sh", "ss",
        "stat", "sysctl", "uname",
    ]
    .into_iter()
    .map(tool)
    .collect();
    let sleep = tool("sleep");
    extra.push(GuestFile { guest: PathBuf::from("/bin/sleep"), ..sleep });
    extra.insert(
        0,
        GuestFile {
            host: tests.binary,
            guest: PathBuf::from(IN_GUEST_TESTS_PATH),
            with_shared_libraries: true,
        },
    );
    extra.push(GuestFile {
        host: kit.forwarder_module.clone(),
        guest: PathBuf::from(WORKSPACE_FORWARDER_GUEST_PATH),
        with_shared_libraries: false,
    });
    let resolver = work.path().join("empty-resolv.conf");
    std::fs::write(&resolver, []).expect("create empty resolver bind target");
    extra.push(GuestFile {
        host: resolver,
        guest: PathBuf::from("/etc/resolv.conf"),
        with_shared_libraries: false,
    });

    let mut modules = vec![LOOPBACK_MODULE_GUEST_PATH.to_owned()];
    for &(relative, guest_path) in MODULES {
        extra.push(
            host_module_file(&kit, work.path(), relative, Path::new(guest_path))
                .unwrap_or_else(|source| panic!("stage {relative}: {source}")),
        );
        modules.push(guest_path.to_owned());
    }
    modules.push(DUMMY_MODULE_GUEST_PATH.to_owned());

    let image = kit
        .image(
            work.path(),
            "control-plane-native-closure-diagnostic",
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
            FWD_GUEST_BOUNDARY.raw(15),
            &work.path().join("run"),
            &ScriptedBootExtra { cmdline: Vec::new(), memory_mib: Some(4_096), vcpus: Some(2) },
        )
        .expect("boot bounded diagnostic guest");

    assert_eq!(
        exec_with_modules(&mut guest, &modules, "/usr/bin/sh", &["-c", SETUP], CELL_BOUND),
        0,
        "guest preconditions"
    );
    let console_before = guest.vm.console().len();
    let code = exec_with_modules(
        &mut guest,
        &modules,
        "/usr/bin/sh",
        &[
            "-c",
            CGROUP_EXEC,
            "control-plane-native",
            IN_GUEST_TESTS_PATH,
            "--test-threads=1",
            "--nocapture",
            "--exact",
            TEST,
        ],
        CELL_BOUND,
    );
    let console = guest.vm.console();
    let run_console = console.get(console_before..).unwrap_or(&console);
    println!("===== control-plane bounded exact replay =====\n{run_console}");
    let _ = guest.take_peer().power_off();
    let exit = guest.vm.wait_exit(Duration::from_secs(30)).expect("wait guest exit");
    assert!(exit.is_some(), "diagnostic guest powers off");
    assert_eq!(code, 0, "fixed negative case exits zero");
    assert!(
        run_console.contains("test result: ok. 1 passed; 0 failed;"),
        "fixed negative case reports one libtest pass"
    );
    assert!(!run_console.contains("SKIP:"), "fixed negative case cannot skip");
    assert!(
        run_console.contains("[diag] DNS53 negative typed error: Bind")
            && run_console.contains("AddrInUse"),
        "fixed case reaches the original typed Bind assertion"
    );
    println!("DNS53_NEGATIVE_TYPED_ASSERTION_PASS");
}
