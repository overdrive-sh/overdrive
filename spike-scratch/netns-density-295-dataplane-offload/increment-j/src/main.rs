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

const TEST: &str =
    "integration::vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c";
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

const DUMMY_BOOT_STATE: &str = r#"
set -eu
module=absent
if [ -e /sys/module/dummy ]; then
  module=loaded
fi
set -- /sys/class/net/*
echo "DUMMY-AUTO diag first command: module=$module runtime_numdummies_file=$([ -e /sys/module/dummy/parameters/numdummies ] && echo present || echo absent) interface_count=$# interfaces=$*"
"#;

const DUMMY_CLEANUP: &str = r#"
set -eu
test -e /sys/module/dummy
test ! -e /sys/module/dummy/parameters/numdummies
echo 'DUMMY-AUTO diag empty_load_params=true source_default_numdummies=1 sysfs_param_permission=0'
set -- /sys/class/net/*
echo "DUMMY-AUTO diag post-load interfaces=$*"
test "$#" -eq 2
test "$1" = /sys/class/net/dummy0
test "$2" = /sys/class/net/lo
link="$(/usr/bin/ip -d -o link show dev dummy0)"
operstate="$(/usr/bin/cat /sys/class/net/dummy0/operstate)"
master=absent
if [ -e /sys/class/net/dummy0/master ]; then master=present; fi
ipv4="$(/usr/bin/ip -4 -o addr show dev dummy0)"
ipv6="$(/usr/bin/ip -6 -o addr show dev dummy0)"
routes4="$(/usr/bin/ip -4 route show table all dev dummy0)"
routes6="$(/usr/bin/ip -6 route show table all dev dummy0)"
echo "DUMMY-AUTO diag link=<$link>"
echo "DUMMY-AUTO diag state: operstate=$operstate master=$master ipv4=<$ipv4> ipv6=<$ipv6> routes4=<$routes4> routes6=<$routes6>"
case "$link" in *' dummy '*) ;; *) exit 1 ;; esac
test "$operstate" = down
test "$master" = absent
test -z "$ipv4$ipv6$routes4$routes6"
/usr/bin/ip link del dev dummy0
set -- /sys/class/net/*
echo "DUMMY-AUTO diag post-cleanup interfaces=$* module=$([ -e /sys/module/dummy ] && echo loaded || echo absent)"
test "$#" -eq 1
test "$1" = /sys/class/net/lo
test -e /sys/module/dummy
echo 'DUMMY_AUTO_DEVICE_PROVEN_AND_REMOVED'
"#;

const STEP13_04_ROUTE_PROOF: &str = r#"
set -eu
test -e /sys/module/dummy
/usr/bin/ip link add ovd-gk-route type dummy
/usr/bin/ip addr add 192.0.2.1/32 dev ovd-gk-route
/usr/bin/ip link set ovd-gk-route up
/usr/bin/ip route add default dev ovd-gk-route src 192.0.2.1 metric 42760
route="$(/usr/bin/ip -4 route get 100.95.255.201)"
case "$route" in *'dev ovd-gk-route'*'src 192.0.2.1'*) ;; *) exit 1 ;; esac
echo "DUMMY-AUTO diag STEP13-04 named-sink route=<$route>"
/usr/bin/ip route del default dev ovd-gk-route metric 42760
/usr/bin/ip link del ovd-gk-route
if /usr/bin/ip -4 route get 100.95.255.201 >/dev/null 2>&1; then exit 1; fi
set -- /sys/class/net/*
test "$#" -eq 1
test "$1" = /sys/class/net/lo
echo 'DUMMY_AUTO_DEVICE_NAMED_SINK_RETAINED_AND_CLEANED'
"#;

const SETUP: &str = r#"
set -eu
/usr/bin/chmod 0660 /dev/kvm /dev/vhost-vsock
/usr/bin/mkdir -p /sys/fs/cgroup /sys/fs/bpf /var /run/netns
/usr/bin/mount -t cgroup2 none /sys/fs/cgroup
/usr/bin/mount -t bpf bpf /sys/fs/bpf
printf '%s\n' '127.0.0.1 localhost' '::1 localhost' > /etc/hosts
printf '%s\n' 'overdrive-cgroup-test' > /proc/sys/kernel/hostname
printf '%s\n' '+cpu +memory +io +pids' > /sys/fs/cgroup/cgroup.subtree_control
test ! -e /var/run
test ! -L /var/run
/usr/bin/ln -s /run /var/run
test -L /var/run
test "$(cd /var/run && pwd -P)" = /run
test -d /var/run/netns
/usr/bin/sysctl -qw net.ipv4.conf.all.route_localnet=0 net.ipv4.conf.default.route_localnet=0 net.ipv4.conf.lo.route_localnet=0
/usr/bin/ip -4 route replace local 100.95.0.1/32 dev lo proto 247
echo 'LEGB-AUDIT diagnostic: setup complete'
"#;

const AUDIT: &str = r#"
set -eu
all="$(/usr/bin/sysctl -n net.ipv4.conf.all.route_localnet)"
default="$(/usr/bin/sysctl -n net.ipv4.conf.default.route_localnet)"
lo="$(/usr/bin/sysctl -n net.ipv4.conf.lo.route_localnet)"
echo "LEGB-AUDIT diag route_localnet=$all:$default:$lo"
test "$all:$default:$lo" = '0:0:0'
gateway="$(/usr/bin/ip -4 route show table local exact 100.95.0.1/32)"
echo "LEGB-AUDIT diag gateway_raw=<$gateway>"
gateway="${gateway% }"
echo "LEGB-AUDIT diag gateway_normalized=<$gateway>"
case "$gateway" in
  'local 100.95.0.1 dev lo proto 247 scope host'|'local 100.95.0.1 dev lo table local proto 247 scope host') ;;
  *) echo "LEGB-AUDIT gateway mismatch: $gateway" >&2; exit 1 ;;
esac
main="$(/usr/bin/ip -4 route show table main)"
echo "LEGB-AUDIT diag main=<$main>"
test -z "$main"
if /usr/bin/ip -4 route get 100.95.255.200 >/dev/null 2>&1; then exit 1; fi
set -- /sys/class/net/*
echo "LEGB-AUDIT diag interfaces=$*"
test "$#" -eq 1
test "$1" = /sys/class/net/lo
rules="$(/usr/bin/ip -4 rule show)"
echo "LEGB-AUDIT diag rules=<$rules>"
case "$rules" in *'fwmark 0x1 lookup 100'*) exit 1 ;; esac
table100="$(/usr/bin/ip -4 route show table 100 2>/dev/null || true)"
echo "LEGB-AUDIT diag table100=<$table100>"
test -z "$table100"
echo "LEGB-AUDIT readback: route_localnet=0 gateway='$gateway' no_default_or_uplink=true table100_clean=true"
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
    let tests = InGuestTests::prepared("overdrive-worker", "integration")
        .expect("prepared control-plane integration binary");
    let work = WorkDir::create(&staging, "control-plane-native-closure-diagnostic")
        .expect("diagnostic workdir");

    let mut extra: Vec<GuestFile> = [
        "cat", "chmod", "ip", "ln", "mkdir", "mount", "nft", "sh", "ss", "stat", "sysctl", "uname",
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
            "legb-audit-diagnostic",
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
            FWD_GUEST_BOUNDARY.raw(6),
            &work.path().join("run"),
            &ScriptedBootExtra { cmdline: Vec::new(), memory_mib: Some(4_096), vcpus: Some(2) },
        )
        .expect("boot bounded diagnostic guest");

    let provenance_before = guest.vm.console().len();
    let boot_state =
        exec_with_modules(&mut guest, &[], "/usr/bin/sh", &["-c", DUMMY_BOOT_STATE], CELL_BOUND);
    let console = guest.vm.console();
    println!(
        "===== dummy boot state =====\n{}",
        console.get(provenance_before..).unwrap_or(&console)
    );
    assert_eq!(boot_state, 0, "read first-observable dummy boot state");
    let cleanup_before = guest.vm.console().len();
    let cleanup_code =
        exec_with_modules(&mut guest, &modules, "/usr/bin/sh", &["-c", DUMMY_CLEANUP], CELL_BOUND);
    let console = guest.vm.console();
    println!(
        "===== dummy automatic-device cleanup =====\n{}",
        console.get(cleanup_before..).unwrap_or(&console)
    );
    assert_eq!(cleanup_code, 0, "dummy module load provenance and automatic-device cleanup");
    assert_eq!(
        exec_with_modules(
            &mut guest,
            &modules,
            "/usr/bin/sh",
            &["-c", STEP13_04_ROUTE_PROOF],
            CELL_BOUND,
        ),
        0,
        "loaded dummy module still creates and cleans the STEP 13-04 named sink"
    );
    let console = guest.vm.console();
    println!(
        "===== dummy automatic-device provenance =====\n{}",
        console.get(provenance_before..).unwrap_or(&console)
    );
    assert_eq!(
        exec_with_modules(&mut guest, &modules, "/usr/bin/sh", &["-c", SETUP], CELL_BOUND),
        0,
        "guest preconditions"
    );
    let audit_before = guest.vm.console().len();
    let audit_code =
        exec_with_modules(&mut guest, &modules, "/usr/bin/sh", &["-c", AUDIT], CELL_BOUND);
    let console = guest.vm.console();
    println!(
        "===== pre-case LEGB audit =====\n{}",
        console.get(audit_before..).unwrap_or(&console)
    );
    assert_eq!(audit_code, 0, "pre-case LEGB audit");
    let console_before = guest.vm.console().len();
    let code = exec_with_modules(
        &mut guest,
        &modules,
        "/usr/bin/sh",
        &[
            "-c",
            CGROUP_EXEC,
            "worker-legb-native",
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
    assert_eq!(
        exec_with_modules(&mut guest, &modules, "/usr/bin/sh", &["-c", AUDIT], CELL_BOUND),
        0,
        "post-case LEGB audit"
    );
    println!("===== LEGB audit guest console =====\n{}", guest.vm.console());
    let _ = guest.take_peer().power_off();
    let exit = guest.vm.wait_exit(Duration::from_secs(30)).expect("wait guest exit");
    assert!(exit.is_some(), "diagnostic guest powers off");
    assert_eq!(code, 0, "unchanged LEGB case exits zero");
    assert!(
        run_console.contains("test result: ok. 1 passed; 0 failed;"),
        "unchanged LEGB case reports one pass"
    );
    assert!(!run_console.contains("SKIP "), "bounded native case cannot skip");
    println!("DUMMY_AUTO_DEVICE_CLEANUP_LEGB_PASS");
}
