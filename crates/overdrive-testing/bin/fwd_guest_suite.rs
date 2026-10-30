//! Runs the existing `overdrive-fwd` integration scenarios, the unchanged
//! `overdrive-dataplane` native integration cells, and the already-authored
//! STEP 13-04, STEP 13-05, STEP 13-06 and STEP 14-01 real-kernel cells inside
//! disposable `GuestKit` Cloud Hypervisor guests. Test-support executable
//! only; never shipped by a production crate.
//!
//! Prepare and run it on the qualified metal launcher without making the
//! physical kernel the module under test:
//!
//! ```text
//! cargo xtask metal run -- cargo xtask guest-kit-prepare
//! cargo xtask metal run -- cargo run -p overdrive-testing \
//!   --bin overdrive-fwd-guest-suite --features integration-tests
//! ```
//!
//! `OVERDRIVE_FWD_GUEST_GROUP` selects one fail-closed isolation group from
//! `probe`, `core`, `unload`, `ulp`, `mtls-relay`, `cgroup-default`, `stress`,
//! and `nested-boundary`. Omitting it runs every group in that order; each
//! isolation boundary gets a fresh guest boot.

#![cfg(target_os = "linux")]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a fail-closed native test runner reports each guest's evidence"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use overdrive_testing::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use overdrive_testing::guest_kit::cid_bands::FWD_GUEST_BOUNDARY;
use overdrive_testing::guest_kit::{
    DUMMY_MODULE_GUEST_PATH, GuestFile, GuestImageSpec, GuestInit, GuestKit, GuestKitError,
    LOOPBACK_MODULE_GUEST_PATH, ScriptedBootExtra, WorkDir, host_module_file,
};
use overdrive_testing::guest_lane::{CELL_BOUND, IN_GUEST_TESTS_PATH, InGuestTests};
use overdrive_testing::vm_fixture::default_staging_root;

const GUEST_MIB: u32 = 4_096;
const TEST_THREADS: &str = "--test-threads=1";
const NO_CAPTURE: &str = "--nocapture";
const ABSENT_LOOPBACK: &str = "integration::probe_gold::an_absent_vsock_loopback_transport_is_refused_naming_stream_round_trip";
const HEALTHY_PROBE: &str = "integration::probe_gold::the_probe_passes_and_leaves_no_residue_on_a_correctly_provisioned_node";
const ULP: &str =
    "integration::unload_in_guest::a_ulp_socket_keeps_the_module_for_the_boot_in_a_real_guest";
const MTLS_RELAY: &str = "integration::mtls_inbound_guest_relay::";
const MTLS_RELAY_BASELINE_BODIES: usize = 7;
const DATAPLANE_NATIVE_BODIES: usize = 36;
const MTLS_RELAY_TOTAL_BODIES: usize = MTLS_RELAY_BASELINE_BODIES + DATAPLANE_NATIVE_BODIES;
const DATAPLANE_LIB_NATIVE_BODIES: usize = 3;
const MTLS_RELAY_COMPLETE_BODIES: usize = MTLS_RELAY_TOTAL_BODIES + DATAPLANE_LIB_NATIVE_BODIES;
const STRESS: &str = "integration::unload_in_guest::the_unload_stress_never_faults_in_a_guest";
const NFNETLINK_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nfnetlink.ko";
const NF_TABLES_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nf_tables.ko";
const VETH_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/veth.ko";
const TLS_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/tls.ko";
const NF_CONNTRACK_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nf_conntrack.ko";
const NF_NAT_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nf_nat.ko";
const NFT_NAT_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nft_nat.ko";
const NFT_CHAIN_NAT_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nft_chain_nat.ko";
const NFT_MASQ_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nft_masq.ko";
const PYTHON_STDLIB_ARCHIVE_GUEST_PATH: &str = "/opt/overdrive-testing/python-stdlib.tar";
const DATAPLANE_LIB_TESTS_PATH: &str = "/opt/overdrive-testing/dataplane-lib-tests";
const NESTED_DEVICE_MODULES: &[(&str, &str)] = &[
    ("drivers/crypto/ccp/ccp", "/lib/overdrive-testing/modules/ccp.ko"),
    ("virt/lib/irqbypass", "/lib/overdrive-testing/modules/irqbypass.ko"),
    ("arch/x86/kvm/kvm", "/lib/overdrive-testing/modules/kvm.ko"),
    ("arch/x86/kvm/kvm-amd", "/lib/overdrive-testing/modules/kvm-amd.ko"),
    ("drivers/vhost/vhost_iotlb", "/lib/overdrive-testing/modules/vhost_iotlb.ko"),
    ("drivers/vhost/vhost", "/lib/overdrive-testing/modules/vhost.ko"),
    ("drivers/vhost/vhost_vsock", "/lib/overdrive-testing/modules/vhost_vsock.ko"),
];
// `nft_tproxy` links both protocol implementations and their defragmentation
// dependencies even though STEP 13-04's owned table is IPv4. Load the exact
// declared dependency closure before its two expression modules; all files
// exist only in the disposable image.
const STEP13_04_NFT_MODULES: &[(&str, &str)] = &[
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
// D26's `fib daddr type` expression adds the IPv4 nft fib evaluator to the
// existing shared-table closure. Keep it out of the unrelated `mtls-relay`
// guest: only the cgroup worker manifest selects the STEP 13-06 bodies.
const STEP13_06_NFT_MODULES: &[(&str, &str)] = &[
    ("net/netfilter/nft_fib", "/lib/overdrive-testing/modules/nft_fib.ko"),
    ("net/ipv4/netfilter/nft_fib_ipv4", "/lib/overdrive-testing/modules/nft_fib_ipv4.ko"),
];
const NESTED_GUEST_KIT_MODULE_SOURCES: &[&str] = &[
    "drivers/net/dummy",
    "net/ipv4/inet_diag",
    "net/ipv4/tcp_diag",
    "net/ipv4/udp_diag",
    "net/vmw_vsock/vsock",
    "net/vmw_vsock/vmw_vsock_virtio_transport_common",
    "net/vmw_vsock/vmw_vsock_virtio_transport",
];
const DATAPLANE_NAT_MODULES: &[(&str, &str)] = &[
    ("net/netfilter/nf_conntrack", NF_CONNTRACK_MODULE_GUEST_PATH),
    ("net/netfilter/nf_nat", NF_NAT_MODULE_GUEST_PATH),
    ("net/netfilter/nft_nat", NFT_NAT_MODULE_GUEST_PATH),
    ("net/netfilter/nft_chain_nat", NFT_CHAIN_NAT_MODULE_GUEST_PATH),
    ("net/netfilter/nft_masq", NFT_MASQ_MODULE_GUEST_PATH),
];
const NESTED_REFLINK_IMAGE: &str = "/var/lib/overdrive-testing/reflink.xfs";
const NESTED_PREBUILT_INIT: &str = "/opt/overdrive-testing/prebuilt-overdrive-init";
const XFS_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/xfs.ko";
const STEP14_01_CELL_BOUND: Duration = Duration::from_secs(600);
const CGROUP_CONTROLLERS: &str = r#"
set -eu
printf '%s\n' '127.0.0.1 localhost' '::1 localhost' > /etc/hosts
hosts="$(/usr/bin/cat /etc/hosts)"
expected_hosts="$(printf '%s\n' '127.0.0.1 localhost' '::1 localhost')"
if [ "$hosts" != "$expected_hosts" ]; then
  echo "cgroup-default guest /etc/hosts mismatch: $hosts" >&2
  exit 1
fi
mounts="$(/usr/bin/cat /proc/mounts)"
case "$mounts" in
  *" /sys/fs/bpf bpf "*) ;;
  *) echo "cgroup-default guest bpffs is not mounted at /sys/fs/bpf" >&2; exit 1 ;;
esac
printf '%s\n' 'overdrive-cgroup-test' > /proc/sys/kernel/hostname
node="$(/usr/bin/cat /proc/sys/kernel/hostname)"
if [ "$node" != 'overdrive-cgroup-test' ]; then
  echo "cgroup-default guest hostname mismatch: $node" >&2
  exit 1
fi
controllers="$(/usr/bin/cat /sys/fs/cgroup/cgroup.controllers)"
for controller in cpu memory io pids; do
  case " $controllers " in
    *" $controller "*) ;;
    *) echo "missing cgroup-v2 controller: $controller" >&2; exit 1 ;;
  esac
done
printf '%s\n' '+cpu +memory +io +pids' > /sys/fs/cgroup/cgroup.subtree_control
enabled="$(/usr/bin/cat /sys/fs/cgroup/cgroup.subtree_control)"
for controller in cpu memory io pids; do
  case " $enabled " in
    *" $controller "*) ;;
    *) echo "cgroup-v2 controller was not enabled for root children: $controller" >&2; exit 1 ;;
  esac
done
"#;
const CGROUP_NOFILE_EXEC: &str = r#"
set -eu
ulimit -Hn 376832
ulimit -Sn 376832
hard="$(ulimit -Hn)"
soft="$(ulimit -Sn)"
if [ "$hard" -lt 376832 ] || [ "$soft" -lt 376832 ]; then
  echo "cgroup-default RLIMIT_NOFILE is below 376832: soft=$soft hard=$hard" >&2
  exit 1
fi
exec "$@"
"#;
const CONTROL_PLANE_NORMAL_NATIVE_SETUP: &str = r#"
set -eu
test -f /etc/resolv.conf
test ! -L /etc/resolv.conf
test ! -s /etc/resolv.conf
/usr/bin/setsid /bin/sleep 0
ownership_file=/run/overdrive-testing-control-plane-gateway-ownership
test ! -e "$ownership_file"
gateway="$(/usr/bin/ip -4 route show table local exact 100.95.0.1/32)"
gateway="${gateway% }"
case "$gateway" in
  '')
    /usr/bin/ip -4 route add local 100.95.0.1/32 dev lo proto 247
    printf '%s\n' created > "$ownership_file"
    ;;
  'local 100.95.0.1 dev lo proto 247 scope host'|'local 100.95.0.1 dev lo table local proto 247 scope host')
    printf '%s\n' borrowed > "$ownership_file"
    ;;
  *) echo "control-plane native gateway route mismatch: $gateway" >&2; exit 1 ;;
esac
gateway="$(/usr/bin/ip -4 route show table local exact 100.95.0.1/32)"
gateway="${gateway% }"
case "$gateway" in
  'local 100.95.0.1 dev lo proto 247 scope host'|'local 100.95.0.1 dev lo table local proto 247 scope host') ;;
  *) echo "control-plane native gateway route readback mismatch: $gateway" >&2; exit 1 ;;
esac
if /usr/bin/ip -4 route get 100.95.255.200 >/dev/null 2>&1; then
  echo 'control-plane native setup unexpectedly routes a non-gateway guest-prefix address' >&2
  exit 1
fi
ownership="$(/usr/bin/cat "$ownership_file")"
echo "CONTROL-PLANE-NATIVE preconditions: resolv_bind_target=empty-ordinary-file setsid_sleep=pass gateway='$gateway' gateway_ownership=$ownership non_gateway_unroutable=true"
"#;
const CONTROL_PLANE_NORMAL_NATIVE_CLEANUP: &str = r#"
set -eu
ownership_file=/run/overdrive-testing-control-plane-gateway-ownership
ownership="$(/usr/bin/cat "$ownership_file")"
gateway="$(/usr/bin/ip -4 route show table local exact 100.95.0.1/32)"
gateway="${gateway% }"
case "$gateway" in
  'local 100.95.0.1 dev lo proto 247 scope host'|'local 100.95.0.1 dev lo table local proto 247 scope host') ;;
  *) echo "control-plane native cleanup gateway route mismatch: $gateway" >&2; exit 1 ;;
esac
case "$ownership" in
  created)
    /usr/bin/ip -4 route del local 100.95.0.1/32 dev lo table local
    test -z "$(/usr/bin/ip -4 route show table local exact 100.95.0.1/32)"
    ;;
  borrowed) ;;
  *) echo "control-plane native cleanup ownership mismatch: $ownership" >&2; exit 1 ;;
esac
if /usr/bin/ip -4 route get 100.95.255.200 >/dev/null 2>&1; then
  echo 'control-plane native cleanup left a route covering the guest prefix' >&2
  exit 1
fi
echo "CONTROL-PLANE-NATIVE cleanup: gateway_ownership=$ownership created_route_absent_or_borrowed_route_preserved=true non_gateway_unroutable=true"
"#;

// Scripted-guest PID 1 loads every staged optional module with empty
// parameters before accepting the first command. The qualified dummy module's
// default is numdummies=1, so boot creates an unconfigured dummy0 before any
// selected body. Prove that first-observable state and remove only that
// fixture-owned automatic device. The module remains loaded so STEP 13-04 can
// create its explicitly named temporary sink.
const DUMMY_AUTO_DEVICE_CLEANUP: &str = r#"
set -eu
test -e /sys/module/dummy
test ! -e /sys/module/dummy/parameters/numdummies
set -- /sys/class/net/*
if [ "$#" -ne 2 ] || [ "$1" != /sys/class/net/dummy0 ] || [ "$2" != /sys/class/net/lo ]; then
  echo "cgroup-default post-load interface set is not the one automatic dummy plus loopback: $*" >&2
  exit 1
fi
link="$(/usr/bin/ip -d -o link show dev dummy0)"
case "$link" in
  *' dummy '*) ;;
  *) echo "cgroup-default automatic dummy0 link kind mismatch: $link" >&2; exit 1 ;;
esac
operstate="$(/usr/bin/cat /sys/class/net/dummy0/operstate)"
master=absent
if [ -e /sys/class/net/dummy0/master ]; then
  master=present
fi
if [ "$operstate" != down ] || [ "$master" != absent ]; then
  echo "cgroup-default automatic dummy0 is configured: operstate=$operstate master=$master" >&2
  exit 1
fi
ipv4="$(/usr/bin/ip -4 -o addr show dev dummy0)"
ipv6="$(/usr/bin/ip -6 -o addr show dev dummy0)"
routes4="$(/usr/bin/ip -4 route show table all dev dummy0)"
routes6="$(/usr/bin/ip -6 route show table all dev dummy0)"
if [ -n "$ipv4$ipv6$routes4$routes6" ]; then
  echo "cgroup-default automatic dummy0 has address/route state: ipv4='$ipv4' ipv6='$ipv6' routes4='$routes4' routes6='$routes6'" >&2
  exit 1
fi
echo "CGROUP-DUMMY-AUTO loaded: empty_load_params=true source_default_numdummies=1 sysfs_param_permission=0 operstate=$operstate master=$master ipv4=none ipv6=none routes=none link='$link'"
/usr/bin/ip link del dev dummy0
set -- /sys/class/net/*
if [ "$#" -ne 1 ] || [ "$1" != /sys/class/net/lo ]; then
  echo "cgroup-default automatic dummy0 cleanup left unexpected interfaces: $*" >&2
  exit 1
fi
test -e /sys/module/dummy
echo 'CGROUP-DUMMY-AUTO cleanup: module=loaded interfaces=lo'
"#;
const DATAPLANE_NATIVE_SETUP: &str = r#"
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
expected_passwd="$(printf '%s\n' 'root:x:0:0:root:/root:/bin/sh' 'tcpdump:x:985:985:tcpdump:/nonexistent:/usr/sbin/nologin')"
expected_group="$(printf '%s\n' 'root:x:0:' 'tcpdump:x:985:')"
test "$(/usr/bin/cat /etc/passwd)" = "$expected_passwd"
test "$(/usr/bin/cat /etc/group)" = "$expected_group"
capture_identity="$(/usr/bin/id tcpdump)"
case "$capture_identity" in
  'uid=985(tcpdump) gid=985(tcpdump) groups=985(tcpdump)') ;;
  *) echo "dataplane-native tcpdump identity mismatch: $capture_identity" >&2; exit 1 ;;
esac
mounts="$(/usr/bin/cat /proc/mounts)"
case "$mounts" in
  *" /sys/fs/cgroup cgroup2 "*) ;;
  *) echo "dataplane-native cgroup2 is not mounted" >&2; exit 1 ;;
esac
case "$mounts" in
  *" /sys/fs/bpf bpf "*) ;;
  *) echo "dataplane-native bpffs is not mounted" >&2; exit 1 ;;
esac
/usr/bin/python3 -c 'import socket,sys,time; assert socket.AF_INET and sys.version_info >= (3, 10)'
/usr/bin/ethtool --version
/usr/bin/bpftool version
/usr/bin/nft --version
/usr/bin/strace -V
/usr/bin/tcpdump --version
bpf_sha256="$(/usr/bin/python3 -c 'import hashlib; print(hashlib.sha256(open("/home/ubuntu/overdrive/target/bpf/overdrive_bpf.o", "rb").read()).hexdigest())')"
echo "DATAPLANE-NATIVE preconditions: cgroup2=mounted bpffs=mounted varrun=alias bpf_artifact=present bpf_sha256=$bpf_sha256 python=socket tools=present tcpdump_identity=$capture_identity"
"#;
// GuestKit has no NIC/default route. The two host-local constant-rule cells
// need a routable main-table lookup to reach nft's output hook. A dummy sink
// gives them that qualified-node precondition without an external packet path;
// the runner removes it before the remote/no-default-route cells.
const STEP13_04_ROUTE_SETUP: &str = r#"
set -eu
if /usr/bin/ip -4 route get 100.95.255.201 >/dev/null 2>&1; then
  echo "STEP13-04 route precondition unexpectedly already resolves 100.95.255.201" >&2
  exit 1
fi
/usr/bin/ip link add ovd-gk-route type dummy
/usr/bin/ip addr add 192.0.2.1/32 dev ovd-gk-route
/usr/bin/ip link set ovd-gk-route up
/usr/bin/ip route add default dev ovd-gk-route src 192.0.2.1 metric 42760
route="$(/usr/bin/ip -4 route get 100.95.255.201)"
case "$route" in
  *"dev ovd-gk-route"*"src 192.0.2.1"*) ;;
  *) echo "STEP13-04 route precondition mismatch: $route" >&2; exit 1 ;;
esac
echo "STEP13-04 route precondition readback: $route"
"#;
const STEP13_04_ROUTE_CLEANUP: &str = r#"
set -eu
/usr/bin/ip route del default dev ovd-gk-route metric 42760
/usr/bin/ip link del ovd-gk-route
if /usr/bin/ip -4 route get 100.95.255.201 >/dev/null 2>&1; then
  echo "STEP13-04 test-owned route survived cleanup" >&2
  exit 1
fi
echo "STEP13-04 route cleanup readback: unroutable"
"#;

// STEP 13-06's off-host control names 192.0.2.1 as its destination, so the
// STEP 13-04 dummy route (whose local source is 192.0.2.1) would turn that
// destination into a host-local address. Give the same bounded no-NIC route
// lifecycle a distinct TEST-NET-2 source and prove TEST-NET-1 stays unicast.
const STEP13_06_ROUTE_SETUP: &str = r#"
set -eu
if /usr/bin/ip -4 route get 192.0.2.1 >/dev/null 2>&1; then
  echo "STEP13-06 route precondition unexpectedly already resolves 192.0.2.1" >&2
  exit 1
fi
test -z "$(/usr/bin/ip -4 route show table local exact 192.0.2.1/32)"
/usr/bin/ip link add ovd-gk-d26 type dummy
/usr/bin/ip addr add 198.51.100.1/32 dev ovd-gk-d26
/usr/bin/ip link set ovd-gk-d26 up
/usr/bin/ip route add default dev ovd-gk-d26 src 198.51.100.1 metric 42761
route="$(/usr/bin/ip -4 route get 192.0.2.1)"
case "$route" in
  *"dev ovd-gk-d26"*"src 198.51.100.1"*) ;;
  *) echo "STEP13-06 route precondition mismatch: $route" >&2; exit 1 ;;
esac
test -z "$(/usr/bin/ip -4 route show table local exact 192.0.2.1/32)"
echo "STEP13-06 route precondition readback: $route destination_type=unicast"
"#;

const STEP13_06_ROUTE_CLEANUP: &str = r#"
set -eu
/usr/bin/ip route del default dev ovd-gk-d26 metric 42761
/usr/bin/ip link del ovd-gk-d26
if /usr/bin/ip -4 route get 192.0.2.1 >/dev/null 2>&1; then
  echo "STEP13-06 test-owned route survived cleanup" >&2
  exit 1
fi
test -z "$(/usr/bin/ip -4 route show table local exact 192.0.2.1/32)"
echo "STEP13-06 route cleanup readback: unroutable destination_not_local=true"
"#;

// V-LEGB-MARK's appliance-faithful namespace has route_localnet disabled, the
// gateway as its only unmarked route into the guest prefix, no node default
// route and no uplink. The existing leg-B bodies install and observe the
// production fwmark/table-100 path themselves. This script establishes only
// their qualified-node precondition after STEP 13-04's temporary default has
// been removed, and fails closed on every read-back.
const STEP13_05_LEGB_SETUP: &str = r#"
set -eu
/usr/bin/sysctl -qw net.ipv4.conf.all.route_localnet=0 net.ipv4.conf.default.route_localnet=0 net.ipv4.conf.lo.route_localnet=0
all="$(/usr/bin/sysctl -n net.ipv4.conf.all.route_localnet)"
default="$(/usr/bin/sysctl -n net.ipv4.conf.default.route_localnet)"
lo="$(/usr/bin/sysctl -n net.ipv4.conf.lo.route_localnet)"
if [ "$all:$default:$lo" != '0:0:0' ]; then
  echo "STEP13-05 route_localnet readback mismatch: all=$all default=$default lo=$lo" >&2
  exit 1
fi
/usr/bin/ip -4 route replace local 100.95.0.1/32 dev lo proto 247
echo "STEP13-05 route_localnet readback: all=$all default=$default lo=$lo"
"#;

const STEP13_05_LEGB_AUDIT: &str = r#"
set -eu
all="$(/usr/bin/sysctl -n net.ipv4.conf.all.route_localnet)"
default="$(/usr/bin/sysctl -n net.ipv4.conf.default.route_localnet)"
lo="$(/usr/bin/sysctl -n net.ipv4.conf.lo.route_localnet)"
if [ "$all:$default:$lo" != '0:0:0' ]; then
  echo "STEP13-05 route_localnet changed: all=$all default=$default lo=$lo" >&2
  exit 1
fi
gateway="$(/usr/bin/ip -4 route show table local exact 100.95.0.1/32)"
gateway="${gateway% }"
case "$gateway" in
  'local 100.95.0.1 dev lo proto 247 scope host'|'local 100.95.0.1 dev lo table local proto 247 scope host') ;;
  *) echo "STEP13-05 gateway local-route mismatch: $gateway" >&2; exit 1 ;;
esac
main="$(/usr/bin/ip -4 route show table main)"
if [ -n "$main" ]; then
  echo "STEP13-05 unexpected main-table route: $main" >&2
  exit 1
fi
if /usr/bin/ip -4 route get 100.95.255.200 >/dev/null 2>&1; then
  echo 'STEP13-05 a route covers the non-gateway guest prefix' >&2
  /usr/bin/ip -4 route get 100.95.255.200 >&2 || true
  exit 1
fi
set -- /sys/class/net/*
if [ "$#" -ne 1 ] || [ "$1" != /sys/class/net/lo ]; then
  echo "STEP13-05 unexpected node uplink/interface set: $*" >&2
  exit 1
fi
rules="$(/usr/bin/ip -4 rule show)"
case "$rules" in
  *'fwmark 0x1 lookup 100'*)
    echo 'STEP13-05 stale fwmark 0x1 lookup 100 rule outside a production body' >&2
    exit 1
    ;;
esac
table100="$(/usr/bin/ip -4 route show table 100 2>/dev/null || true)"
if [ -n "$table100" ]; then
  echo "STEP13-05 stale table-100 route outside a production body: $table100" >&2
  exit 1
fi
echo "STEP13-05 qualified-node readback: route_localnet=0 gateway='$gateway' no_default_or_uplink=true table100_clean=true"
"#;

type Cell = (&'static str, usize, bool);
type CgroupManifests<'a> = (
    &'a [Cell],
    &'a [Cell],
    &'a [Cell],
    &'a [Cell],
    &'a [Cell],
    &'a [Cell],
    &'a [Cell],
    &'a [Cell],
);

/// The 36 active real-kernel dataplane bodies in the approved three-package
/// inventory. The seven existing inbound guest-relay bodies remain the
/// separately counted baseline of the same existing `mtls-relay` group.
const DATAPLANE_NATIVE: &[Cell] = &[
    ("integration::atomic_swap::atomic_swap_under_50kpps_traffic_drops_zero_packets", 1, true),
    ("integration::atomic_swap::hash_of_maps_handle_create_swap_delete_round_trip", 1, true),
    (
        "integration::atomic_swap::kernel_rejects_inner_map_alloc_existing_mapping_preserved",
        1,
        true,
    ),
    ("integration::atomic_swap::removing_backend_purges_orphaned_backend_map_entries", 1, true),
    (
        "integration::atomic_swap::swap_inner_map_distributes_traffic_across_new_backend_set",
        1,
        true,
    ),
    (
        "integration::atomic_swap::verifier_budget_xdp_reverse_nat_lookup_within_absolute_ceiling",
        1,
        true,
    ),
    (
        "integration::atomic_swap::verifier_budget_xdp_service_map_lookup_within_20pct_of_baseline",
        1,
        true,
    ),
    (
        "integration::deregister_retry_safety::deregister_purges_reverse_entry_when_forward_already_removed",
        1,
        true,
    ),
    (
        "integration::local_backend_proto_connect::tcp_and_udp_connect_to_same_vip_port_reach_proto_correct_backend",
        1,
        true,
    ),
    ("integration::maglev_real::maglev_real_distribution_under_xdp_trafficgen", 1, true),
    ("integration::mtls_agent_handshake::agent_handshake_presents_held_svid_server_role", 1, true),
    (
        "integration::mtls_guardrails::guardrails_fail_closed_limits_supervision_exemption_authn_boundary",
        1,
        true,
    ),
    (
        "integration::mtls_inbound_enforce::inbound_enforce_origdst_server_mtls_ktls_rx_splice_to_server",
        1,
        true,
    ),
    (
        "integration::multi_listener_tcp_udp_e2e::adding_listener_on_resubmit_converges_without_breaking_existing",
        1,
        true,
    ),
    (
        "integration::multi_listener_tcp_udp_e2e::each_listener_reverse_path_independently_vip_sourced",
        1,
        true,
    ),
    (
        "integration::multi_listener_tcp_udp_e2e::empty_backend_purge_of_one_listener_preserves_co_resident_proto",
        1,
        true,
    ),
    (
        "integration::multi_listener_tcp_udp_e2e::two_listener_service_installs_both_protocol_paths",
        1,
        true,
    ),
    (
        "integration::redirect_neigh_attach::loader_attaches_xdp_reverse_nat_on_backend_veth_and_retires_tc_egress",
        1,
        true,
    ),
    (
        "integration::reverse_nat_e2e::empty_backend_deregistration_releases_allocator_memo_entries",
        1,
        true,
    ),
    (
        "integration::reverse_nat_e2e::empty_backend_purge_scoped_to_frontend_leaves_other_ports_alive",
        1,
        true,
    ),
    (
        "integration::reverse_nat_e2e::empty_backend_update_removes_service_map_and_reverse_nat_entries",
        1,
        true,
    ),
    (
        "integration::reverse_nat_e2e::real_tcp_connection_completes_through_vip_with_payload_echo",
        1,
        true,
    ),
    (
        "integration::reverse_nat_e2e::removing_backend_purges_reverse_nat_entry_no_stale_rewrite",
        1,
        true,
    ),
    ("integration::reverse_nat_udp_e2e::every_udp_reply_independently_source_rewritten", 1, true),
    (
        "integration::reverse_nat_udp_e2e::missing_backend_response_distinguished_from_wrong_source",
        1,
        true,
    ),
    (
        "integration::reverse_nat_udp_e2e::single_udp_listener_round_trip_carries_vip_source",
        1,
        true,
    ),
    (
        "integration::sanity_mixed_batch::mixed_batch_increments_per_class_counters_correctly",
        1,
        true,
    ),
    (
        "integration::service_map_forward::ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth",
        1,
        true,
    ),
    ("integration::service_map_vip_port::service_map_keyed_on_vip_port_not_backend_port", 1, true),
    (
        "integration::unconnected_udp_reply_hardening::below_floor_kernel_refuses_at_attach_preflight_observably",
        1,
        true,
    ),
    (
        "integration::unconnected_udp_reply_hardening::non_service_unconnected_udp_reads_real_source_recvmsg4_noop_on_miss",
        1,
        true,
    ),
    (
        "integration::unconnected_udp_roundtrip::forward_and_reverse_map_entries_present_after_one_register",
        1,
        true,
    ),
    (
        "integration::unconnected_udp_roundtrip::kernel_reply_source_meets_tier1_reply_mirror_at_backend_identity",
        1,
        true,
    ),
    (
        "integration::unconnected_udp_roundtrip::second_unconnected_query_reuses_same_mapping_statelessly",
        1,
        true,
    ),
    (
        "integration::unconnected_udp_roundtrip::unconnected_sendto_recvfrom_reads_vip_sourced_reply",
        1,
        true,
    ),
    ("integration::veth_attach::missing_iface_returns_typed_iface_not_found_error", 1, true),
];

/// Active dataplane library bodies whose successful normal-run summaries were
/// either a `root/CAP_BPF` bailout or a real-map permission failure.
const DATAPLANE_LIB_NATIVE: &[Cell] = &[
    (
        "maps::hash_of_maps::tests::alloc_failure_with_zero_inner_entries_yields_typed_variant",
        1,
        true,
    ),
    ("maps::hash_of_maps::tests::create_outer_and_inner_then_atomic_swap", 1, true),
    ("maps::reverse_local_map_handle::real_map_tests::remove_deletes_the_reverse_entry", 1, true),
];

#[derive(Clone, Copy)]
struct CgroupRunSupport<'a> {
    manifests: CgroupManifests<'a>,
    stage_step13_04_nft: bool,
    stage_step13_05_legb: bool,
    stage_normal_gate_native: bool,
}

const CGROUP_ACCEPTANCE: &[Cell] = &[
    (
        "acceptance::job_stop_idempotent::stop_on_already_stopped_job_returns_already_stopped_outcome",
        1,
        true,
    ),
    ("acceptance::job_stop_intent_key::stop_writes_separate_intent_key_preserving_spec", 1, true),
    ("acceptance::job_stop_unknown::stop_on_unknown_job_returns_404", 1, true),
];

const CGROUP_CONTROL_PLANE: &[Cell] = &[
    (
        "integration::backend_discovery_bridge::boot_composition::attach_mode_fallback_emits_structured_event_on_dummy_iface",
        1,
        true,
    ),
    (
        "integration::backend_discovery_bridge::boot_composition::boot_composes_ebpf_dataplane_and_attaches_xdp_to_both_ifaces",
        1,
        true,
    ),
    (
        "integration::backend_discovery_bridge::boot_composition::boot_refuses_when_client_iface_does_not_exist",
        1,
        true,
    ),
    (
        "integration::backend_discovery_bridge::boot_composition::boot_refuses_when_earned_trust_probe_fails",
        1,
        true,
    ),
    (
        "integration::backend_discovery_bridge::boot_composition::boot_refuses_when_iface_has_no_ipv4_address",
        1,
        true,
    ),
    (
        "integration::backend_discovery_bridge::boot_composition::boot_resolves_host_ipv4_via_getifaddrs_on_client_iface",
        1,
        true,
    ),
    (
        "integration::backend_discovery_bridge::boot_composition::boot_succeeds_when_earned_trust_probe_round_trips_backend_map",
        1,
        true,
    ),
    (
        "integration::backend_discovery_bridge::boot_composition::graceful_shutdown_detaches_xdp_and_removes_bpffs_pin",
        1,
        true,
    ),
    (
        "integration::built_in_ca_operator_composition::serve_persistent_ca::refuse_to_start_does_not_remint_the_root",
        1,
        true,
    ),
    (
        "integration::built_in_ca_operator_composition::serve_persistent_ca::serve_first_boot_generates_seals_and_persists_root",
        1,
        true,
    ),
    (
        "integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refuses_on_absent_kek",
        1,
        true,
    ),
    (
        "integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refuses_on_corrupted_envelope",
        1,
        true,
    ),
    (
        "integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refuses_on_wrong_kek_material",
        1,
        true,
    ),
    (
        "integration::built_in_ca_operator_composition::serve_persistent_ca::serve_restart_adopts_same_root_no_remint",
        1,
        true,
    ),
    (
        "integration::built_in_ca_operator_composition::serve_persistent_ca::serve_refusal_causes_are_pairwise_distinct",
        1,
        true,
    ),
    (
        "integration::cgroup_isolation::alloc_scope_has_writable_cpu_weight_and_memory_max::",
        2,
        false,
    ),
    ("integration::cgroup_isolation::subtree_control_delegation_is_idempotent::", 1, false),
    ("integration::concurrent_submit_toctou::", 2, false),
    (
        "integration::describe_round_trip::get_v1_jobs_id_returns_described_job_after_submit",
        1,
        true,
    ),
    (
        "integration::describe_round_trip::get_v1_jobs_malformed_id_returns_400_with_field_id",
        1,
        true,
    ),
    (
        "integration::describe_round_trip::get_v1_jobs_unknown_id_returns_404_with_error_body",
        1,
        true,
    ),
    (
        "integration::describe_round_trip::describe_spec_digest_equals_content_hash_of_archived_bytes",
        1,
        true,
    ),
    (
        "integration::describe_round_trip::describe_returns_spec_digest_matching_submit_response",
        1,
        true,
    ),
    ("integration::describe_round_trip::submit_then_describe_round_trips_spec_and_digest", 1, true),
    ("integration::idempotent_resubmit::", 5, false),
    ("integration::serve_kernel_assigned_vsock_ports::", 1, false),
    (
        "integration::server_lifecycle::run_server_binds_on_ephemeral_port_and_reports_bound_address",
        1,
        true,
    ),
    (
        "integration::server_lifecycle::reqwest_client_with_minted_ca_gets_200_on_v1_cluster_info",
        1,
        true,
    ),
    ("integration::server_lifecycle::response_alpn_negotiation_is_http_2", 1, true),
    (
        "integration::server_lifecycle::cancellation_token_shutdown_drains_in_flight_request",
        1,
        true,
    ),
    ("integration::server_lifecycle::all_adr_0008_paths_return_200_on_stub_router", 1, true),
    (
        "integration::shared_guest_network_startup::serve_boots_through_the_pinned_order_with_the_guest_network_composed",
        1,
        true,
    ),
    (
        "integration::shared_guest_network_startup::an_rlimit_nofile_below_the_owner_budget_refuses_boot",
        1,
        true,
    ),
    ("integration::shared_guest_network_startup::a_failed_beacon_bind_refuses_boot", 1, true),
    ("integration::submit_round_trip::", 5, false),
];

const CGROUP_WORKER: &[Cell] = &[
    (
        "integration::cgroup_manager::write_to_readonly_cgroup_file::write_to_readonly_cgroup_events_propagates_real_eacces",
        1,
        true,
    ),
    (
        "integration::real_cgroup_fs::cgroup_kill_terminates_pids::cgroup_kill_terminates_every_pid_within_two_seconds",
        1,
        true,
    ),
    (
        "integration::real_cgroup_fs::controller_validation::cpu_weight_out_of_range_returns_einval",
        1,
        true,
    ),
    (
        "integration::real_cgroup_fs::procs_pid_movement::writing_pid_to_cgroup_procs_moves_the_process_into_scope",
        1,
        true,
    ),
    (
        "integration::real_cgroup_fs::pseudo_file_synthesis::cgroup_events_appears_at_mkdir_time",
        1,
        true,
    ),
    (
        "integration::real_cgroup_fs::rmdir_auto_reap::empty_scope_rmdir_succeeds_kernel_reaps_pseudo_files",
        1,
        true,
    ),
    (
        "integration::real_cgroup_fs::subtree_control_ebusy::no_internal_process_rule_returns_ebusy_dispatches_to_typed_variant",
        1,
        true,
    ),
];

/// Existing STEP 13-04 control-plane bodies whose kernel/cgroup/netns effects
/// must execute inside the disposable guest. These extend the existing
/// `cgroup-default` selection; they are not another selectable group or lane.
const STEP13_04_CONTROL_PLANE: &[Cell] = &[
    (
        "integration::boot_member_clear_refusal::a_rejected_boot_member_clear_refuses_the_composed_boot_with_its_own_cause",
        1,
        true,
    ),
    (
        "integration::boot_member_clear_refusal::a_boot_member_clear_that_leaves_members_refuses_with_the_members_it_observed",
        1,
        true,
    ),
    (
        "integration::boot_member_clear_refusal::a_boot_member_clear_that_commits_after_the_refusal_publishes_nothing",
        1,
        true,
    ),
    (
        "integration::shared_element_cleanup_failure::shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address",
        1,
        true,
    ),
    (
        "integration::shared_element_cleanup_failure::shared_element_cleanup_failure_readback_failed_retains_retirement_and_address",
        1,
        true,
    ),
    (
        "integration::shared_guest_network_startup::a_failed_step8_table_write_refuses_boot_and_the_restart_converges",
        1,
        true,
    ),
];

/// Existing STEP 13-04 worker bodies whose real nft/netns/socket effects must
/// execute inside the disposable guest. Exact filters keep every body and the
/// expected count explicit while reusing the staged integration binary.
const STEP13_04_WORKER: &[Cell] = &[
    (
        "integration::mtls_intercept_install::shared_program_absence_create_readback_idempotence_and_guard_drop",
        1,
        true,
    ),
    (
        "integration::mtls_intercept_install::shared_program_replaces_only_listener_targets_and_preserves_foreign_complement",
        1,
        true,
    ),
    (
        "integration::mtls_intercept_install::shared_program_valid_wrong_target_observation_is_non_mutating",
        1,
        true,
    ),
    (
        "integration::netns_density_shared_owner::stopping_one_shared_allocation_preserves_the_unrelated_handle_and_complete_listener_owner",
        1,
        true,
    ),
    (
        "integration::netns_density_shared_owner::member_loss_is_an_ipsets_failure_and_repair_restores_exactly_the_member",
        1,
        true,
    ),
    (
        "integration::netns_density_shared_owner::policy_route_loss_is_repaired_with_live_members_and_the_prior_guard_is_relinquished",
        1,
        true,
    ),
    (
        "integration::netns_density_shared_owner::a_differently_targeted_program_is_never_rewritten",
        1,
        true,
    ),
    (
        "integration::shared_intercept_members::each_deleted_intercept_object_is_restored_exactly_with_live_allocations",
        1,
        true,
    ),
];

/// The two host-local constant-prefix cells require a real main-table default
/// route so local output reaches the nft output hook. The runner owns that
/// disposable route only around these exact cells.
const STEP13_04_HOST_LOCAL_FIREWALL: &[Cell] = &[
    (
        "integration::vsock_constant_guest_prefix_rules::a_host_local_connection_to_a_guest_prefix_address_is_rejected_at_once",
        1,
        true,
    ),
    (
        "integration::vsock_constant_guest_prefix_rules::a_host_local_datagram_to_a_guest_prefix_address_fails_eperm",
        1,
        true,
    ),
];

/// The remote-drop and leg-C cells run after the test-owned default route is
/// removed, preserving their no-node-default-route environment.
const STEP13_04_REMOTE_FIREWALL: &[Cell] = &[
    (
        "integration::vsock_constant_guest_prefix_rules::a_remote_connection_to_an_unassigned_guest_prefix_address_is_dropped",
        1,
        true,
    ),
    (
        "integration::vsock_constant_guest_prefix_rules::a_leg_c_diverted_remote_connection_reaches_leg_c_both_ways",
        1,
        true,
    ),
];

/// The two already-authored STEP 13-05 production-composition refusals. Their
/// ordinary package run reaches the real cgroup bootstrap before either
/// prefix assertion; the existing `cgroup-default` guest supplies that
/// prerequisite without changing either body.
const STEP13_05_CONTROL_PLANE: &[Cell] = &[
    (
        "integration::shared_guest_network_startup::a_vip_range_overlapping_the_guest_prefix_refuses_boot_before_any_effect",
        1,
        true,
    ),
    (
        "integration::shared_guest_network_startup::a_node_address_inside_the_guest_prefix_refuses_boot_naming_the_interface",
        1,
        true,
    ),
];

/// The three already-authored V-LEGB-MARK cells. They run after the STEP
/// 13-04 host-output-only default route has been removed, inside `GuestKit`'s
/// no-NIC disposable guest. The bodies retain their production mark/source,
/// exact socket, outside-prefix and unregistered-tuple oracles.
const STEP13_05_WORKER: &[Cell] = &[
    (
        "integration::vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c",
        1,
        true,
    ),
    (
        "integration::vsock_legb_dial::a_dial_to_a_peer_outside_the_guest_prefix_is_neither_marked_nor_bound",
        1,
        true,
    ),
    (
        "integration::vsock_legb_dial::a_marked_dial_to_an_unregistered_tuple_is_refused_and_reaches_no_host_socket",
        1,
        true,
    ),
];

/// The ten STEP 13-06 D26 cells whose authored topology needs no fixture
/// default route. Every body boots the production shared table and retains
/// its original socket, nft-order, reachability and cleanup assertions.
const STEP13_06_WORKER: &[Cell] = &[
    (
        "integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_loopback_service_is_refused_econnrefused_without_hang",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_host_interface_address_is_refused",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_udp_first_transmit_to_a_local_service_fails_eperm",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_udp_socket_reusing_an_unmarked_flows_source_port_is_still_rejected",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_a_subnet_directed_broadcast_reaches_no_service",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_link_local_service_is_refused",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_neighbour_stays_reachable",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_packet_carrying_the_owner_bit_among_other_bits_is_refused",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_packet_carrying_only_the_leg_s_or_tproxy_mark_reaches_a_local_service",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_connect_storm_across_repeated_table_replacements_reaches_no_local_service",
        1,
        true,
    ),
];

/// The authored limited-broadcast, multicast and off-host controls require a
/// disposable main-table default so their first UDP transmit reaches the
/// output hook. Reuse the existing bounded dummy-route lifecycle around these
/// exact bodies only.
const STEP13_06_ROUTED_WORKER: &[Cell] = &[
    (
        "integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_the_limited_broadcast_reaches_no_service",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_the_all_hosts_multicast_group_is_rejected_eperm",
        1,
        true,
    ),
    (
        "integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_an_off_host_address_is_not_rejected",
        1,
        true,
    ),
];

/// Active control-plane integration bodies whose normal-run result was a
/// root/CAP bailout or a real privileged substrate failure. They extend the
/// existing control-plane guest after the retained 55+18+5 boundary.
const CONTROL_PLANE_NORMAL_NATIVE: &[Cell] = &[
    (
        "integration::adopt_on_restart::serve_restart_readopts_surviving_slot_and_gcs_orphan_netns",
        1,
        true,
    ),
    (
        "integration::adopt_on_restart::serve_restart_sweeps_surviving_per_workload_tproxy_rule",
        1,
        true,
    ),
    (
        "integration::dns_responder_bind::empty_fallback_binds_zero_sockets_and_warns_it_is_deaf",
        1,
        true,
    ),
    (
        "integration::dns_responder_bind::per_gateway_addr_fallback_binds_when_wildcard_is_held",
        1,
        true,
    ),
    ("integration::dns_responder_bind::probe_refuses_when_no_bindable_port", 1, true),
    ("integration::dns_responder_bind::probe_refuses_when_store_unreadable_at_listseed", 1, true),
    (
        "integration::dns_responder_bind::run_server_refuses_boot_when_the_guest_dns_probe_fails_through_the_required_port",
        1,
        true,
    ),
    (
        "integration::dns_responder_bind::the_responder_audit_reads_back_its_socket_and_fails_after_loss",
        1,
        true,
    ),
    (
        "integration::dns_responder_bind::wildcard_bind_answers_frontend_and_source_pins_reply",
        1,
        true,
    ),
    (
        "integration::serve_boot_provisions_veth::serve_boot_attaches_two_distinct_xdp_programs_to_two_distinct_veths",
        1,
        true,
    ),
    (
        "integration::serve_boot_provisions_veth::single_iface_collision_surfaces_typed_iface_xdp_slot_busy",
        1,
        true,
    ),
    (
        "integration::veth_provision_idempotent::provision_complete_pair_converges_to_silent_success",
        1,
        true,
    ),
    ("integration::veth_provision_idempotent::provision_completes_half_provisioned_pair", 1, true),
    ("integration::veth_provision_idempotent::provision_creates_pair_when_absent", 1, true),
    (
        "integration::veth_provision_idempotent::provision_disables_tx_offload_on_both_ends_and_is_idempotent",
        1,
        true,
    ),
    (
        "integration::veth_provision_idempotent::provision_recreates_pair_when_client_absent_but_peer_present",
        1,
        true,
    ),
    ("integration::veth_provision_idempotent::provision_recreates_pair_when_peer_absent", 1, true),
    (
        "integration::veth_provision_idempotent::provision_repairs_tx_offload_drifted_back_on",
        1,
        true,
    ),
    (
        "integration::workload_netns_provision::provision_creates_and_idempotently_converges_per_workload_netns",
        1,
        true,
    ),
    (
        "integration::workload_netns_provision::provision_injects_node_local_responder_into_netns_resolv_conf",
        1,
        true,
    ),
];

/// The five already-authored STEP 14-01 control-plane cells. Each starts the
/// production `run_server` composition and launches real workload VMs. The
/// outer test process therefore runs in the existing `cgroup-default`
/// disposable guest while its workload VMs are nested Cloud Hypervisor guests.
const STEP14_01_CONTROL_PLANE: &[Cell] = &[
    (
        "integration::guest_flow_through_serve::the_forwarder_inventory_counts_every_held_guest_flow_through_serve",
        1,
        true,
    ),
    (
        "integration::guest_flow_through_serve::a_killed_launchs_attachment_leaves_the_owner_and_the_next_launch_is_accepted",
        1,
        true,
    ),
    (
        "integration::guest_flow_through_serve::the_forwarder_inventory_counts_probe_and_relay_watches_through_serve",
        1,
        true,
    ),
    (
        "integration::guest_flow_through_serve::a_dropped_probe_stream_is_an_abort_at_the_guest_through_serve",
        1,
        true,
    ),
    (
        "integration::guest_flow_through_serve::a_vmm_exit_under_a_held_relay_stream_resets_the_client_through_serve",
        1,
        true,
    ),
];

/// The already-authored STEP 14-01 CLI walking skeleton, through the public
/// deploy handler and the production `serve` composition.
const STEP14_01_CLI: &[Cell] = &[(
    "integration::vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy",
    1,
    true,
)];

const CGROUP_DEFAULT_BASELINE_BODIES: usize = 55;
const STEP13_04_NATIVE_BODIES: usize = 18;
const STEP13_05_NATIVE_BODIES: usize = 5;
const STEP13_06_NATIVE_BODIES: usize = 13;
const CGROUP_DEFAULT_TOTAL_BODIES: usize =
    CGROUP_DEFAULT_BASELINE_BODIES + STEP13_04_NATIVE_BODIES + STEP13_05_NATIVE_BODIES;
const CONTROL_PLANE_NORMAL_NATIVE_BODIES: usize = 20;
const CGROUP_DEFAULT_RETAINED_BODIES: usize =
    CGROUP_DEFAULT_TOTAL_BODIES + CONTROL_PLANE_NORMAL_NATIVE_BODIES;
const CGROUP_DEFAULT_COMPLETE_BODIES: usize =
    CGROUP_DEFAULT_RETAINED_BODIES + STEP13_06_NATIVE_BODIES;
const STEP14_01_CONTROL_PLANE_BODIES: usize = 5;
const STEP14_01_CLI_BODIES: usize = 1;
const STEP14_01_NATIVE_BODIES: usize = STEP14_01_CONTROL_PLANE_BODIES + STEP14_01_CLI_BODIES;

struct GuestRun {
    guest: overdrive_testing::guest_kit::ScriptedGuest,
    module_paths: Vec<String>,
    _work: WorkDir,
}

fn reports_runtime_skip(console: &str) -> bool {
    const PREFIXES: &[&str] = &["[skip]", "SKIP ", "SKIP:", "skip:", "(skip)"];
    console.lines().any(|line| {
        let line = line.trim_start();
        let diagnostic = line
            .split_once(" ... ")
            .map_or(line, |(_, after_test_name)| after_test_name.trim_start());
        PREFIXES.iter().any(|prefix| diagnostic.starts_with(prefix))
    })
}

impl GuestRun {
    fn boot(
        kit: &GuestKit,
        tests: &InGuestTests,
        name: &str,
        slot: u32,
        extra: Vec<GuestFile>,
        module_paths: Vec<String>,
    ) -> Result<Self, GuestKitError> {
        Self::boot_with_vcpus(kit, tests, name, slot, extra, module_paths, 2)
    }

    fn boot_with_vcpus(
        kit: &GuestKit,
        tests: &InGuestTests,
        name: &str,
        slot: u32,
        mut extra: Vec<GuestFile>,
        module_paths: Vec<String>,
        vcpus: u8,
    ) -> Result<Self, GuestKitError> {
        let staging = default_staging_root();
        let work = WorkDir::create(&staging, &format!("fwd-suite-{name}"))?;
        extra.insert(
            0,
            GuestFile {
                host: tests.binary.clone(),
                guest: PathBuf::from(IN_GUEST_TESTS_PATH),
                with_shared_libraries: true,
            },
        );
        let image = kit.image(
            work.path(),
            name,
            &GuestImageSpec {
                init: GuestInit::ScriptedPeer,
                forwarder_module: true,
                loopback_module: true,
                extra,
            },
        )?;
        let guest = kit.boot_scripted_with(
            &image,
            FWD_GUEST_BOUNDARY.raw(slot),
            &work.path().join("run"),
            &ScriptedBootExtra {
                cmdline: Vec::new(),
                memory_mib: Some(GUEST_MIB),
                vcpus: Some(vcpus),
            },
        )?;
        Ok(Self { guest, module_paths, _work: work })
    }

    fn exec(
        &mut self,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
    ) -> Result<(), GuestKitError> {
        self.exec_with_skip_policy(IN_GUEST_TESTS_PATH, test_args, expected_passed, bound, false)
    }

    fn exec_required(
        &mut self,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
    ) -> Result<(), GuestKitError> {
        self.exec_with_skip_policy(IN_GUEST_TESTS_PATH, test_args, expected_passed, bound, true)
    }

    fn exec_binary_required(
        &mut self,
        program: &str,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
    ) -> Result<(), GuestKitError> {
        self.exec_with_skip_policy(program, test_args, expected_passed, bound, true)
    }

    fn exec_with_skip_policy(
        &mut self,
        program: &str,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
        forbid_runtime_skip: bool,
    ) -> Result<(), GuestKitError> {
        let console_before = self.guest.vm.console().len();
        let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES];
        command.push(LOOPBACK_MODULE_GUEST_PATH);
        command.extend(self.module_paths.iter().map(String::as_str));
        command.extend(["--", program]);
        command.extend_from_slice(test_args);
        let code = match self.guest.peer().exec(&command, bound) {
            Ok(code) => code,
            Err(source) => {
                return Err(GuestKitError::Staging {
                    detail: format!(
                        "guest test argv {test_args:?} did not return within {bound:?}"
                    ),
                    source: std::io::Error::other(format!(
                        "{source}; disposable guest console tail:\n{}",
                        self.guest.vm.console_tail(16 * 1024)
                    )),
                });
            }
        };
        let console = self.guest.vm.console();
        let run_console = console.get(console_before..).unwrap_or(&console);
        if code != 0 {
            return Err(GuestKitError::Staging {
                detail: format!("guest test argv {test_args:?} exited {code}"),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        let summary = format!("test result: ok. {expected_passed} passed; 0 failed;");
        if !run_console.contains(&summary) {
            return Err(GuestKitError::Staging {
                detail: format!(
                    "guest test argv {test_args:?} exited zero without the required {summary:?}"
                ),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        if forbid_runtime_skip && reports_runtime_skip(run_console) {
            return Err(GuestKitError::Staging {
                detail: format!(
                    "required guest test argv {test_args:?} reported a runtime infrastructure skip"
                ),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        Ok(())
    }

    fn exec_cgroup(
        &mut self,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
    ) -> Result<(), GuestKitError> {
        self.exec_cgroup_with_skip_policy(test_args, expected_passed, bound, false)
    }

    fn exec_cgroup_required(
        &mut self,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
    ) -> Result<(), GuestKitError> {
        self.exec_cgroup_with_skip_policy(test_args, expected_passed, bound, true)
    }

    fn exec_cgroup_with_skip_policy(
        &mut self,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
        forbid_runtime_skip: bool,
    ) -> Result<(), GuestKitError> {
        let console_before = self.guest.vm.console().len();
        let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES];
        command.push(LOOPBACK_MODULE_GUEST_PATH);
        command.extend(self.module_paths.iter().map(String::as_str));
        command.extend([
            "--",
            "/usr/bin/sh",
            "-c",
            CGROUP_NOFILE_EXEC,
            "cgroup-default",
            IN_GUEST_TESTS_PATH,
        ]);
        command.extend_from_slice(test_args);
        let code = match self.guest.peer().exec(&command, bound) {
            Ok(code) => code,
            Err(source) => {
                return Err(GuestKitError::Staging {
                    detail: format!(
                        "cgroup guest test argv {test_args:?} did not return within {bound:?}"
                    ),
                    source: std::io::Error::other(format!(
                        "{source}; disposable guest console tail:\n{}",
                        self.guest.vm.console_tail(16 * 1024)
                    )),
                });
            }
        };
        let console = self.guest.vm.console();
        let run_console = console.get(console_before..).unwrap_or(&console);
        if code != 0 {
            return Err(GuestKitError::Staging {
                detail: format!("cgroup guest test argv {test_args:?} exited {code}"),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        let summary = format!("test result: ok. {expected_passed} passed; 0 failed;");
        if !run_console.contains(&summary) {
            return Err(GuestKitError::Staging {
                detail: format!(
                    "cgroup guest test argv {test_args:?} exited zero without the required {summary:?}"
                ),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        if forbid_runtime_skip && reports_runtime_skip(run_console) {
            return Err(GuestKitError::Staging {
                detail: format!(
                    "required cgroup guest test argv {test_args:?} reported a runtime infrastructure skip"
                ),
                source: std::io::Error::other(run_console.to_owned()),
            });
        }
        Ok(())
    }

    fn command_with_modules(
        &mut self,
        modules: &[String],
        program: &str,
        args: &[&str],
    ) -> Result<(), GuestKitError> {
        let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES];
        command.extend(modules.iter().map(String::as_str));
        command.extend(["--", program]);
        command.extend_from_slice(args);
        self.command(&command)
    }

    fn command(&mut self, argv: &[&str]) -> Result<(), GuestKitError> {
        let code = self.guest.peer().exec(argv, Duration::from_secs(30))?;
        if code != 0 {
            return Err(GuestKitError::Staging {
                detail: format!("guest evidence command {argv:?} exited {code}"),
                source: std::io::Error::other(self.guest.vm.console_tail(4096)),
            });
        }
        Ok(())
    }

    fn finish(mut self, name: &str) -> Result<(), GuestKitError> {
        let console = self.guest.vm.console();
        println!("===== disposable guest {name} =====\n{console}");
        let _ = self.guest.take_peer().power_off();
        let exited = self.guest.vm.wait_exit(Duration::from_secs(30))?;
        if exited.is_none() {
            return Err(GuestKitError::Wait {
                cid: self.guest.vm.cid(),
                source: std::io::Error::other("the disposable guest did not power off"),
            });
        }
        Ok(())
    }
}

fn tool(name: &str) -> GuestFile {
    let host = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
        .into_iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("required guest-suite tool {name} is not installed"));
    GuestFile { host, guest: Path::new("/usr/bin").join(name), with_shared_libraries: true }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("overdrive-testing has a workspace-root grandparent"))
        .to_path_buf()
}

fn workspace_bpf_artifact() -> GuestFile {
    let host = workspace_root().join("target/bpf/overdrive_bpf.o");
    GuestFile { guest: host.clone(), host, with_shared_libraries: false }
}

fn python_stdlib_archive(dir: &Path) -> Result<GuestFile, GuestKitError> {
    let output = Command::new("python3")
        .args(["-c", "import sysconfig; print(sysconfig.get_path('stdlib'), end='')"])
        .output()
        .map_err(|source| GuestKitError::Staging {
            detail: "locating the qualified runner's Python standard library".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(GuestKitError::Staging {
            detail: format!("python3 stdlib query exited {:?}", output.status.code()),
            source: std::io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }
    let stdlib = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let relative =
        stdlib.strip_prefix(Path::new("/")).map_err(|source| GuestKitError::Staging {
            detail: format!("Python stdlib path {} is not absolute", stdlib.display()),
            source: std::io::Error::other(source.to_string()),
        })?;
    if !stdlib.is_dir() {
        return Err(GuestKitError::Staging {
            detail: format!("Python stdlib path {} is not a directory", stdlib.display()),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
    }

    let archive = dir.join("python-stdlib.tar");
    let tar = Command::new("tar")
        .args(["-cf"])
        .arg(&archive)
        .arg("-C")
        .arg("/")
        .arg(relative)
        .output()
        .map_err(|source| GuestKitError::Staging {
            detail: format!("archiving Python stdlib at {}", stdlib.display()),
            source,
        })?;
    if !tar.status.success() {
        return Err(GuestKitError::Staging {
            detail: format!(
                "tar of Python stdlib {} exited {:?}",
                stdlib.display(),
                tar.status.code()
            ),
            source: std::io::Error::other(String::from_utf8_lossy(&tar.stderr).into_owned()),
        });
    }
    Ok(GuestFile {
        host: archive,
        guest: PathBuf::from(PYTHON_STDLIB_ARCHIVE_GUEST_PATH),
        with_shared_libraries: false,
    })
}

fn tcpdump_account_files(dir: &Path) -> Result<[GuestFile; 2], GuestKitError> {
    let passwd = dir.join("tcpdump-passwd");
    let group = dir.join("tcpdump-group");
    fs::write(
        &passwd,
        "root:x:0:0:root:/root:/bin/sh\ntcpdump:x:985:985:tcpdump:/nonexistent:/usr/sbin/nologin\n",
    )
    .map_err(|source| GuestKitError::Staging {
        detail: format!("writing guest tcpdump passwd file at {}", passwd.display()),
        source,
    })?;
    fs::write(&group, "root:x:0:\ntcpdump:x:985:\n").map_err(|source| GuestKitError::Staging {
        detail: format!("writing guest tcpdump group file at {}", group.display()),
        source,
    })?;
    Ok([
        GuestFile {
            host: passwd,
            guest: PathBuf::from("/etc/passwd"),
            with_shared_libraries: false,
        },
        GuestFile { host: group, guest: PathBuf::from("/etc/group"), with_shared_libraries: false },
    ])
}

fn dataplane_native_tools() -> Vec<GuestFile> {
    [
        "bpftool", "cat", "ethtool", "id", "ip", "kill", "ln", "mkdir", "mount", "nc", "nft",
        "python3", "sh", "ss", "strace", "sysctl", "tar", "tcpdump", "true", "uname",
    ]
    .into_iter()
    .map(tool)
    .collect()
}

fn base_tools() -> Vec<GuestFile> {
    ["cat", "cp", "echo", "ip", "nft", "sleep", "ss"].into_iter().map(tool).collect()
}

fn workspace_forwarder_module(kit: &GuestKit) -> GuestFile {
    GuestFile {
        host: kit.forwarder_module.clone(),
        guest: PathBuf::from(format!(
            "/home/ubuntu/overdrive/target/kmod/{}/ovd_fwd.ko",
            kit.kernel_release
        )),
        with_shared_libraries: false,
    }
}

fn cgroup_tools(
    kit: &GuestKit,
    support_dir: &Path,
    stage_step13_04_nft: bool,
    stage_step13_05_legb: bool,
    stage_normal_gate_native: bool,
) -> Result<Vec<GuestFile>, GuestKitError> {
    let mut files: Vec<_> =
        ["cat", "chmod", "ip", "mkdir", "mount", "sh", "stat"].into_iter().map(tool).collect();
    if stage_step13_04_nft {
        files.extend(["ln", "nft", "ss", "uname"].into_iter().map(tool));
    }
    if stage_step13_05_legb {
        // The V-LEGB-MARK fixture pins and reads back route_localnet=0. The
        // five selected bodies call no Python interpreter; do not stage a
        // language runtime that none of their real-I/O paths executes.
        files.push(tool("sysctl"));
    }
    if stage_normal_gate_native {
        files.extend(["ethtool", "setsid", "sysctl"].into_iter().map(tool));
        let resolver = support_dir.join("empty-resolv.conf");
        fs::write(&resolver, []).map_err(|source| GuestKitError::Staging {
            detail: format!("creating guest resolver bind target at {}", resolver.display()),
            source,
        })?;
        files.push(GuestFile {
            host: resolver,
            guest: PathBuf::from("/etc/resolv.conf"),
            with_shared_libraries: false,
        });
    }
    let sleep = tool("sleep");
    files.push(GuestFile { guest: PathBuf::from("/bin/sleep"), ..sleep });
    // The unchanged bodies run the existing FWD-PREREQ identity check at its
    // workspace contract path. Stage the same GuestKit-verified module image
    // there; the image also keeps its normal /lib/overdrive copy and guest
    // boot owns the load. Nothing touches the physical host module.
    files.push(workspace_forwarder_module(kit));
    Ok(files)
}

fn make_reflink_volume(dir: &Path) -> Result<PathBuf, GuestKitError> {
    let image = dir.join("nested-reflink.xfs");
    std::fs::File::create(&image).and_then(|file| file.set_len(768 * 1024 * 1024)).map_err(
        |source| GuestKitError::Staging {
            detail: format!("creating nested reflink volume at {}", image.display()),
            source,
        },
    )?;
    let output = Command::new("mkfs.xfs")
        .args(["-f", "-q", "-m", "reflink=1"])
        .arg(&image)
        .output()
        .map_err(|source| GuestKitError::Staging {
            detail: "spawning mkfs.xfs for the nested GuestKit volume".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(GuestKitError::Staging {
            detail: format!("mkfs.xfs exited {:?}", output.status.code()),
            source: std::io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }
    Ok(image)
}

fn nested_guest_kit_module_source(
    kit: &GuestKit,
    dir: &Path,
    relative: &str,
) -> Result<GuestFile, GuestKitError> {
    let base = format!("/lib/modules/{}/kernel/{relative}.ko", kit.kernel_release);
    let compressed = PathBuf::from(format!("{base}.zst"));
    let guest = compressed.clone();
    let host = if compressed.is_file() {
        compressed
    } else {
        let plain = PathBuf::from(&base);
        if !plain.is_file() {
            return Err(GuestKitError::Staging {
                detail: format!(
                    "the qualified kernel ships no STEP 14-01 nested GuestKit module source {relative}"
                ),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }
        let staged = dir.join(format!("{}.ko.zst", relative.replace('/', "-")));
        let output = Command::new("zstd")
            .args(["-f", "-q"])
            .arg(&plain)
            .arg("-o")
            .arg(&staged)
            .output()
            .map_err(|source| GuestKitError::Staging {
                detail: format!("compressing nested GuestKit module source {relative}"),
                source,
            })?;
        if !output.status.success() {
            return Err(GuestKitError::Staging {
                detail: format!(
                    "compressing nested GuestKit module source {relative} exited {:?}",
                    output.status.code()
                ),
                source: std::io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()),
            });
        }
        staged
    };
    Ok(GuestFile { host, guest, with_shared_libraries: false })
}

/// Stage the exact outer-guest substrate the already-authored STEP 14-01
/// binaries consume. This is the existing GuestKit-inside-GuestKit shape from
/// `nested-boundary`, plus the production serve dependencies those binaries
/// reach: the BPF object, netlink/nft modules, veth, kTLS and the ordinary
/// host tools. It adds no test hook or alternate production composition.
fn step14_01_nested_support(
    kit: &GuestKit,
    dir: &Path,
) -> Result<(Vec<GuestFile>, Vec<String>), GuestKitError> {
    let mut extra: Vec<_> = [
        "cat",
        "chmod",
        "cp",
        "ethtool",
        "ip",
        "ln",
        "losetup",
        "mkdir",
        "mkfs.ext4",
        "mknod",
        "mount",
        "nft",
        "prlimit",
        "setpriv",
        "setsid",
        "sh",
        "ss",
        "stat",
        "sysctl",
        "true",
        "uname",
        "zstd",
    ]
    .into_iter()
    .map(tool)
    .collect();
    let sleep = tool("sleep");
    extra.push(GuestFile { guest: PathBuf::from("/bin/sleep"), ..sleep });
    extra.push(tool("systemd-detect-virt"));
    extra.push(workspace_forwarder_module(kit));
    extra.push(workspace_bpf_artifact());
    for relative in NESTED_GUEST_KIT_MODULE_SOURCES {
        extra.push(nested_guest_kit_module_source(kit, dir, relative)?);
    }

    let resolver = dir.join("empty-resolv.conf");
    fs::write(&resolver, []).map_err(|source| GuestKitError::Staging {
        detail: format!("creating STEP 14-01 guest resolver target at {}", resolver.display()),
        source,
    })?;
    extra.push(GuestFile {
        host: resolver,
        guest: PathBuf::from("/etc/resolv.conf"),
        with_shared_libraries: false,
    });

    let reflink = make_reflink_volume(dir)?;
    extra.push(GuestFile {
        host: reflink,
        guest: PathBuf::from(NESTED_REFLINK_IMAGE),
        with_shared_libraries: false,
    });
    extra.push(GuestFile {
        host: kit.init_bin.clone(),
        guest: PathBuf::from(NESTED_PREBUILT_INIT),
        with_shared_libraries: false,
    });
    extra.push(GuestFile {
        host: kit.kernel_path.clone(),
        guest: PathBuf::from(format!("/boot/vmlinuz-{}", kit.kernel_release)),
        with_shared_libraries: false,
    });
    extra.push(GuestFile {
        host: kit.cloud_hypervisor_bin.clone(),
        guest: PathBuf::from("/usr/bin/cloud-hypervisor"),
        with_shared_libraries: true,
    });

    let mut module_paths = Vec::new();
    let mut stage_module = |relative: &str, guest_path: &str| -> Result<(), GuestKitError> {
        extra.push(host_module_file(kit, dir, relative, Path::new(guest_path))?);
        module_paths.push(guest_path.to_owned());
        Ok(())
    };
    stage_module("fs/xfs/xfs", XFS_MODULE_GUEST_PATH)?;
    for &(relative, guest_path) in NESTED_DEVICE_MODULES {
        stage_module(relative, guest_path)?;
    }
    for &(relative, guest_path) in &[
        ("net/netfilter/nfnetlink", NFNETLINK_MODULE_GUEST_PATH),
        ("net/netfilter/nf_tables", NF_TABLES_MODULE_GUEST_PATH),
        ("drivers/net/veth", VETH_MODULE_GUEST_PATH),
        ("net/tls/tls", TLS_MODULE_GUEST_PATH),
    ] {
        stage_module(relative, guest_path)?;
    }
    for &(relative, guest_path) in STEP13_04_NFT_MODULES {
        stage_module(relative, guest_path)?;
    }
    for &(relative, guest_path) in STEP13_06_NFT_MODULES {
        stage_module(relative, guest_path)?;
    }
    Ok((extra, module_paths))
}

const fn exact(name: &str) -> [&str; 4] {
    [TEST_THREADS, NO_CAPTURE, "--exact", name]
}

fn run_probe_isolation(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let mut guest = GuestRun::boot(kit, tests, "probe-isolation", 7, base_tools(), Vec::new())?;
    guest.command(&["/usr/bin/echo", "FWD-SUITE-BEFORE-REFS"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/ovd_fwd/refcnt"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/vsock_loopback/refcnt"])?;
    guest.command(&["/usr/bin/ss", "-H", "-a", "-f", "vsock"])?;
    guest.exec(&exact(ABSENT_LOOPBACK), 1, CELL_BOUND)?;
    guest.exec(&exact(HEALTHY_PROBE), 1, CELL_BOUND)?;
    guest.command(&["/usr/bin/echo", "FWD-SUITE-AFTER-REFS"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/ovd_fwd/refcnt"])?;
    guest.command(&["/usr/bin/cat", "/sys/module/vsock_loopback/refcnt"])?;
    guest.command(&["/usr/bin/ss", "-H", "-a", "-f", "vsock"])?;
    guest.finish("probe-isolation")
}

fn run_core(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let staging = default_staging_root();
    let modules = WorkDir::create(&staging, "fwd-suite-core-modules")?;
    let module_specs = [
        ("net/netfilter/nfnetlink", NFNETLINK_MODULE_GUEST_PATH),
        ("net/netfilter/nf_tables", NF_TABLES_MODULE_GUEST_PATH),
        ("net/ipv4/netfilter/nf_reject_ipv4", "/lib/overdrive-testing/modules/nf_reject_ipv4.ko"),
        ("net/ipv6/netfilter/nf_reject_ipv6", "/lib/overdrive-testing/modules/nf_reject_ipv6.ko"),
        ("net/netfilter/nft_reject", "/lib/overdrive-testing/modules/nft_reject.ko"),
        ("net/netfilter/nft_reject_inet", "/lib/overdrive-testing/modules/nft_reject_inet.ko"),
        ("net/tls/tls", TLS_MODULE_GUEST_PATH),
    ];
    let mut extra = base_tools();
    let mut module_paths = Vec::new();
    for (relative, guest_path) in module_specs {
        extra.push(host_module_file(kit, modules.path(), relative, Path::new(guest_path))?);
        module_paths.push(guest_path.to_owned());
    }
    let mut guest = GuestRun::boot(kit, tests, "core", 8, extra, module_paths)?;
    guest.exec(
        &[
            TEST_THREADS,
            NO_CAPTURE,
            "--skip",
            ABSENT_LOOPBACK,
            "--skip",
            "integration::guest_boundary::",
            "--skip",
            "integration::unload::",
            "--skip",
            "integration::unload_in_guest::",
        ],
        86,
        Duration::from_secs(900),
    )?;
    guest.finish("core")
}

fn run_unload(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let workspace_module = workspace_forwarder_module(kit);
    let mut guest = GuestRun::boot(kit, tests, "unload", 9, vec![workspace_module], Vec::new())?;
    guest.exec(&[TEST_THREADS, NO_CAPTURE, "integration::unload::"], 4, CELL_BOUND)?;
    guest.finish("unload")
}

fn run_ulp(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let staging = default_staging_root();
    let work = WorkDir::create(&staging, "fwd-suite-tls-module")?;
    let tls = host_module_file(kit, work.path(), "net/tls/tls", Path::new(TLS_MODULE_GUEST_PATH))?;
    let mut guest =
        GuestRun::boot(kit, tests, "ulp", 10, vec![tls], vec![TLS_MODULE_GUEST_PATH.to_owned()])?;
    guest.exec(&exact(ULP), 1, CELL_BOUND)?;
    drop(work);
    guest.finish("ulp")
}

fn run_mtls_relay(kit: &GuestKit, _fwd_tests: &InGuestTests) -> Result<(), GuestKitError> {
    let tests = InGuestTests::prepared("overdrive-dataplane", "integration")?;
    let lib_tests = InGuestTests::prepared("overdrive-dataplane", "overdrive_dataplane")?;
    let staging = default_staging_root();
    let work = WorkDir::create(&staging, "fwd-suite-mtls-relay-module")?;
    let mut extra = dataplane_native_tools();
    extra.push(GuestFile {
        host: lib_tests.binary,
        guest: PathBuf::from(DATAPLANE_LIB_TESTS_PATH),
        with_shared_libraries: true,
    });
    extra.push(workspace_bpf_artifact());
    extra.push(python_stdlib_archive(work.path())?);
    extra.extend(tcpdump_account_files(work.path())?);
    let tls = host_module_file(kit, work.path(), "net/tls/tls", Path::new(TLS_MODULE_GUEST_PATH))?;
    extra.push(tls);
    let module_specs = [
        ("net/netfilter/nfnetlink", NFNETLINK_MODULE_GUEST_PATH),
        ("net/netfilter/nf_tables", NF_TABLES_MODULE_GUEST_PATH),
        ("drivers/net/veth", VETH_MODULE_GUEST_PATH),
        ("net/ipv4/netfilter/nf_defrag_ipv4", STEP13_04_NFT_MODULES[0].1),
        ("net/ipv6/netfilter/nf_defrag_ipv6", STEP13_04_NFT_MODULES[1].1),
        DATAPLANE_NAT_MODULES[0],
        DATAPLANE_NAT_MODULES[1],
        ("net/ipv4/netfilter/nf_reject_ipv4", STEP13_04_NFT_MODULES[2].1),
        ("net/ipv6/netfilter/nf_reject_ipv6", STEP13_04_NFT_MODULES[3].1),
        ("net/ipv4/netfilter/nf_tproxy_ipv4", STEP13_04_NFT_MODULES[4].1),
        ("net/ipv6/netfilter/nf_tproxy_ipv6", STEP13_04_NFT_MODULES[5].1),
        ("net/netfilter/nft_reject", STEP13_04_NFT_MODULES[6].1),
        ("net/ipv4/netfilter/nft_reject_ipv4", STEP13_04_NFT_MODULES[7].1),
        ("net/netfilter/nft_tproxy", STEP13_04_NFT_MODULES[8].1),
        DATAPLANE_NAT_MODULES[2],
        DATAPLANE_NAT_MODULES[3],
        DATAPLANE_NAT_MODULES[4],
    ];
    let mut native_module_paths = Vec::new();
    for (relative, guest_path) in module_specs {
        extra.push(host_module_file(kit, work.path(), relative, Path::new(guest_path))?);
        native_module_paths.push(guest_path.to_owned());
    }
    let mut guest = GuestRun::boot(
        kit,
        &tests,
        "mtls-relay",
        13,
        extra,
        vec![TLS_MODULE_GUEST_PATH.to_owned()],
    )?;
    guest.exec_required(
        &[TEST_THREADS, NO_CAPTURE, MTLS_RELAY],
        MTLS_RELAY_BASELINE_BODIES,
        CELL_BOUND,
    )?;
    let loaded = native_module_paths.clone();
    guest.command_with_modules(&loaded, "/usr/bin/true", &[])?;
    guest.module_paths.extend(native_module_paths);
    guest.command(&["/usr/bin/sh", "-c", DATAPLANE_NATIVE_SETUP])?;
    let native_passed = exec_required_cells(&mut guest, DATAPLANE_NATIVE)?;
    if native_passed != DATAPLANE_NATIVE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "mtls-relay dataplane-native extension selected {native_passed} passing bodies, expected exactly {DATAPLANE_NATIVE_BODIES}"
            ),
            source: std::io::Error::other("the dataplane-native manifest changed"),
        });
    }
    let total_passed = MTLS_RELAY_BASELINE_BODIES + native_passed;
    if total_passed != MTLS_RELAY_TOTAL_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "mtls-relay selected {total_passed} passing bodies in total, expected exactly {MTLS_RELAY_TOTAL_BODIES}"
            ),
            source: std::io::Error::other("the combined mtls-relay manifest changed"),
        });
    }
    let lib_native_passed =
        exec_required_cells_at(&mut guest, DATAPLANE_LIB_TESTS_PATH, DATAPLANE_LIB_NATIVE)?;
    if lib_native_passed != DATAPLANE_LIB_NATIVE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "mtls-relay dataplane-lib extension selected {lib_native_passed} passing bodies, expected exactly {DATAPLANE_LIB_NATIVE_BODIES}"
            ),
            source: std::io::Error::other("the dataplane-lib native manifest changed"),
        });
    }
    let complete_passed = total_passed + lib_native_passed;
    if complete_passed != MTLS_RELAY_COMPLETE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "mtls-relay selected {complete_passed} passing bodies after the library extension, expected exactly {MTLS_RELAY_COMPLETE_BODIES}"
            ),
            source: std::io::Error::other("the complete mtls-relay manifest changed"),
        });
    }
    drop(work);
    guest.finish("mtls-relay")
}

fn exec_required_cells(guest: &mut GuestRun, cells: &[Cell]) -> Result<usize, GuestKitError> {
    exec_required_cells_at(guest, IN_GUEST_TESTS_PATH, cells)
}

fn exec_required_cells_at(
    guest: &mut GuestRun,
    program: &str,
    cells: &[Cell],
) -> Result<usize, GuestKitError> {
    let mut passed = 0;
    for &(filter, expected, is_exact) in cells {
        if is_exact {
            guest.exec_binary_required(program, &exact(filter), expected, CELL_BOUND)?;
        } else {
            guest.exec_binary_required(
                program,
                &[TEST_THREADS, NO_CAPTURE, filter],
                expected,
                CELL_BOUND,
            )?;
        }
        passed += expected;
    }
    Ok(passed)
}

fn exec_cells(guest: &mut GuestRun, cells: &[Cell]) -> Result<usize, GuestKitError> {
    let mut passed = 0;
    for &(filter, expected, is_exact) in cells {
        if is_exact {
            guest.exec_cgroup(&exact(filter), expected, CELL_BOUND)?;
        } else {
            guest.exec_cgroup(&[TEST_THREADS, NO_CAPTURE, filter], expected, CELL_BOUND)?;
        }
        passed += expected;
    }
    Ok(passed)
}

fn exec_step13_05_legb_cells(guest: &mut GuestRun, cells: &[Cell]) -> Result<usize, GuestKitError> {
    if cells.is_empty() {
        return Ok(0);
    }
    guest.command(&["/usr/bin/sh", "-c", STEP13_05_LEGB_SETUP])?;
    let mut passed = 0;
    for &(filter, expected, is_exact) in cells {
        guest.command(&["/usr/bin/sh", "-c", STEP13_05_LEGB_AUDIT])?;
        if is_exact {
            guest.exec_cgroup(&exact(filter), expected, CELL_BOUND)?;
        } else {
            guest.exec_cgroup(&[TEST_THREADS, NO_CAPTURE, filter], expected, CELL_BOUND)?;
        }
        passed += expected;
        guest.command(&["/usr/bin/sh", "-c", STEP13_05_LEGB_AUDIT])?;
    }
    Ok(passed)
}

fn exec_required_cgroup_cells(
    guest: &mut GuestRun,
    cells: &[Cell],
) -> Result<usize, GuestKitError> {
    exec_required_cgroup_cells_with_bound(guest, cells, CELL_BOUND)
}

fn exec_required_cgroup_cells_with_bound(
    guest: &mut GuestRun,
    cells: &[Cell],
    bound: Duration,
) -> Result<usize, GuestKitError> {
    let mut passed = 0;
    for &(filter, expected, is_exact) in cells {
        if is_exact {
            guest.exec_cgroup_required(&exact(filter), expected, bound)?;
        } else {
            guest.exec_cgroup_required(&[TEST_THREADS, NO_CAPTURE, filter], expected, bound)?;
        }
        passed += expected;
    }
    Ok(passed)
}

/// Run one STEP 14-01 test binary in a fresh member of the existing
/// `cgroup-default` disposable-guest group. The production test body launches
/// the inner workload VMs; this outer guest contains every kernel, cgroup,
/// netns, nft, BPF and nested-KVM effect.
fn run_step14_01_binary(
    kit: &GuestKit,
    package: &str,
    target: &str,
    name: &str,
    slot: u32,
    cells: &[Cell],
) -> Result<usize, GuestKitError> {
    let tests = InGuestTests::prepared(package, target)?;
    let staging = default_staging_root();
    let support = WorkDir::create(&staging, &format!("fwd-suite-{name}-support"))?;
    let (extra, module_paths) = step14_01_nested_support(kit, support.path())?;
    let mut guest = GuestRun::boot_with_vcpus(kit, &tests, name, slot, extra, module_paths, 8)?;

    let modules = guest.module_paths.clone();
    guest.command_with_modules(&modules, "/usr/bin/true", &[])?;
    guest.command(&["/usr/bin/chmod", "0660", "/dev/kvm", "/dev/vhost-vsock"])?;
    guest.command(&[
        "/usr/bin/stat",
        "-c",
        "STEP14-01-NESTED-DEVICE %n uid=%u gid=%g mode=%a",
        "/dev/kvm",
        "/dev/vhost-vsock",
    ])?;
    guest.command(&["/usr/bin/systemd-detect-virt"])?;
    guest.command(&[
        "/usr/bin/mkdir",
        "-p",
        "/sys/fs/cgroup",
        "/sys/fs/bpf",
        "/sys/kernel/security",
        "/srv/vm",
    ])?;
    guest.command(&["/usr/bin/mount", "-t", "cgroup2", "none", "/sys/fs/cgroup"])?;
    guest.command(&["/usr/bin/mount", "-t", "bpf", "bpf", "/sys/fs/bpf"])?;
    guest.command(&["/usr/bin/mount", "-t", "securityfs", "securityfs", "/sys/kernel/security"])?;
    guest.command(&["/usr/bin/cat", "/sys/kernel/security/lsm"])?;
    guest.command(&["/usr/bin/sh", "-c", CGROUP_CONTROLLERS])?;
    guest.command(&[
        "/usr/bin/sh",
        "-c",
        "set -eu; mkdir -p /var /run/netns; test ! -e /var/run; test ! -L /var/run; ln -s /run /var/run; test -L /var/run; test \"$(cd /var/run && pwd -P)\" = /run; test -d /var/run/netns",
    ])?;
    guest.command(&["/usr/bin/losetup", "/dev/loop0", NESTED_REFLINK_IMAGE])?;
    guest.command(&["/usr/bin/mount", "-t", "xfs", "/dev/loop0", "/srv/vm"])?;
    guest.command(&["/usr/bin/cp", GUEST_PEER_GUEST_PATH, "/srv/vm/reflink-probe"])?;
    guest.command(&[
        "/usr/bin/cp",
        "--reflink=always",
        "/srv/vm/reflink-probe",
        "/srv/vm/reflink-probe.clone",
    ])?;

    let passed = exec_required_cgroup_cells_with_bound(&mut guest, cells, STEP14_01_CELL_BOUND)?;
    guest.finish(name)?;
    drop(support);
    Ok(passed)
}

#[allow(
    clippy::too_many_lines,
    reason = "one guest lifecycle keeps module staging, private mounts, ordered manifests and fail-closed cleanup together"
)]
fn run_cgroup_binary(
    kit: &GuestKit,
    package: &str,
    target: &str,
    name: &str,
    slot: u32,
    support: CgroupRunSupport<'_>,
) -> Result<(usize, usize, usize, usize, usize), GuestKitError> {
    let CgroupRunSupport {
        manifests,
        stage_step13_04_nft,
        stage_step13_05_legb,
        stage_normal_gate_native,
    } = support;
    let (
        baseline_cells,
        step13_04_cells,
        routed_step13_04_cells,
        post_route_step13_04_cells,
        step13_05_cells,
        step13_06_cells,
        routed_step13_06_cells,
        normal_gate_native_cells,
    ) = manifests;
    let stage_step13_06_nft = !step13_06_cells.is_empty() || !routed_step13_06_cells.is_empty();
    let tests = InGuestTests::prepared(package, target)?;
    let staging = default_staging_root();
    let modules = WorkDir::create(&staging, &format!("fwd-suite-{name}-kernel-modules"))?;
    let mut extra = cgroup_tools(
        kit,
        modules.path(),
        stage_step13_04_nft,
        stage_step13_05_legb,
        stage_normal_gate_native,
    )?;
    let mut module_paths = Vec::new();
    for &(relative, guest_path) in NESTED_DEVICE_MODULES {
        extra.push(host_module_file(kit, modules.path(), relative, Path::new(guest_path))?);
        module_paths.push(guest_path.to_owned());
    }
    let nfnetlink = host_module_file(
        kit,
        modules.path(),
        "net/netfilter/nfnetlink",
        Path::new(NFNETLINK_MODULE_GUEST_PATH),
    )?;
    let nf_tables = host_module_file(
        kit,
        modules.path(),
        "net/netfilter/nf_tables",
        Path::new(NF_TABLES_MODULE_GUEST_PATH),
    )?;
    let veth = host_module_file(
        kit,
        modules.path(),
        "drivers/net/veth",
        Path::new(VETH_MODULE_GUEST_PATH),
    )?;
    let tls =
        host_module_file(kit, modules.path(), "net/tls/tls", Path::new(TLS_MODULE_GUEST_PATH))?;
    extra.push(nfnetlink);
    extra.push(nf_tables);
    extra.push(veth);
    extra.push(tls);
    module_paths.extend([
        NFNETLINK_MODULE_GUEST_PATH.to_owned(),
        NF_TABLES_MODULE_GUEST_PATH.to_owned(),
        VETH_MODULE_GUEST_PATH.to_owned(),
        TLS_MODULE_GUEST_PATH.to_owned(),
    ]);
    if stage_step13_04_nft {
        for &(relative, guest_path) in STEP13_04_NFT_MODULES {
            extra.push(host_module_file(kit, modules.path(), relative, Path::new(guest_path))?);
            module_paths.push(guest_path.to_owned());
        }
        module_paths.push(DUMMY_MODULE_GUEST_PATH.to_owned());
    }
    if stage_step13_06_nft {
        for &(relative, guest_path) in STEP13_06_NFT_MODULES {
            extra.push(host_module_file(kit, modules.path(), relative, Path::new(guest_path))?);
            module_paths.push(guest_path.to_owned());
        }
    }
    let mut guest = GuestRun::boot(kit, &tests, name, slot, extra, module_paths)?;
    let guest_modules = guest.module_paths.clone();
    guest.command_with_modules(&guest_modules, "/usr/bin/sh", &["-c", "true"])?;
    if stage_step13_04_nft {
        guest.command(&["/usr/bin/sh", "-c", DUMMY_AUTO_DEVICE_CLEANUP])?;
    }
    guest.command(&["/usr/bin/chmod", "0660", "/dev/kvm", "/dev/vhost-vsock"])?;
    guest.command(&[
        "/usr/bin/stat",
        "-c",
        "CGROUP-DEVICE %n uid=%u gid=%g mode=%a",
        "/dev/kvm",
        "/dev/vhost-vsock",
    ])?;
    guest.command(&["/usr/bin/mkdir", "-p", "/sys/fs/cgroup", "/sys/fs/bpf"])?;
    guest.command(&["/usr/bin/mount", "-t", "cgroup2", "none", "/sys/fs/cgroup"])?;
    guest.command(&["/usr/bin/mount", "-t", "bpf", "bpf", "/sys/fs/bpf"])?;
    guest.command(&["/usr/bin/sh", "-c", CGROUP_CONTROLLERS])?;
    if stage_step13_04_nft {
        // `ip netns add` publishes handles under /run/netns while the
        // existing PeerNetns fixture opens the traditional /var/run/netns
        // spelling. Keep that authored fixture unchanged inside the
        // disposable guest by installing the standard compatibility symlink.
        guest.command(&[
            "/usr/bin/sh",
            "-c",
            "set -eu; mkdir -p /var /run/netns; test ! -e /var/run; test ! -L /var/run; ln -s /run /var/run; test -L /var/run; test \"$(cd /var/run && pwd -P)\" = /run; test -d /var/run/netns",
        ])?;
    }

    let baseline_passed = exec_cells(&mut guest, baseline_cells)?;
    let mut step13_04_passed = exec_cells(&mut guest, step13_04_cells)?;
    if !routed_step13_04_cells.is_empty() {
        guest.command(&["/usr/bin/sh", "-c", STEP13_04_ROUTE_SETUP])?;
        let routed = exec_cells(&mut guest, routed_step13_04_cells);
        let cleanup = guest.command(&["/usr/bin/sh", "-c", STEP13_04_ROUTE_CLEANUP]);
        match routed {
            Ok(passed) => {
                cleanup?;
                step13_04_passed += passed;
            }
            Err(error) => {
                // Preserve the authored cell's failure; the explicit cleanup
                // was attempted, and dropping the disposable guest is the
                // fail-closed cleanup if that attempt also failed.
                let _ = cleanup;
                return Err(error);
            }
        }
    }
    step13_04_passed += exec_cells(&mut guest, post_route_step13_04_cells)?;
    let step13_05_passed = if stage_step13_05_legb {
        exec_step13_05_legb_cells(&mut guest, step13_05_cells)?
    } else {
        exec_cells(&mut guest, step13_05_cells)?
    };
    let mut step13_06_passed = exec_required_cgroup_cells(&mut guest, step13_06_cells)?;
    if !routed_step13_06_cells.is_empty() {
        guest.command(&["/usr/bin/sh", "-c", STEP13_06_ROUTE_SETUP])?;
        let routed = exec_required_cgroup_cells(&mut guest, routed_step13_06_cells);
        let cleanup = guest.command(&["/usr/bin/sh", "-c", STEP13_06_ROUTE_CLEANUP]);
        match routed {
            Ok(passed) => {
                cleanup?;
                step13_06_passed += passed;
            }
            Err(error) => {
                let _ = cleanup;
                return Err(error);
            }
        }
    }
    let normal_gate_native_passed = if stage_normal_gate_native {
        guest.command(&["/usr/bin/sh", "-c", CONTROL_PLANE_NORMAL_NATIVE_SETUP])?;
        let native = exec_required_cgroup_cells(&mut guest, normal_gate_native_cells);
        let cleanup = guest.command(&["/usr/bin/sh", "-c", CONTROL_PLANE_NORMAL_NATIVE_CLEANUP]);
        match native {
            Ok(passed) => {
                cleanup?;
                passed
            }
            Err(error) => {
                let _ = cleanup;
                return Err(error);
            }
        }
    } else {
        exec_required_cgroup_cells(&mut guest, normal_gate_native_cells)?
    };
    guest.finish(name)?;
    drop(modules);
    Ok((
        baseline_passed,
        step13_04_passed,
        step13_05_passed,
        step13_06_passed,
        normal_gate_native_passed,
    ))
}

#[allow(
    clippy::too_many_lines,
    reason = "the existing group keeps each historical subcount and the new native-covered count independently fail-closed"
)]
fn run_cgroup_default(kit: &GuestKit, _fwd_tests: &InGuestTests) -> Result<(), GuestKitError> {
    let acceptance = run_cgroup_binary(
        kit,
        "overdrive-control-plane",
        "acceptance",
        "cgroup-control-plane-acceptance",
        14,
        CgroupRunSupport {
            manifests: (CGROUP_ACCEPTANCE, &[], &[], &[], &[], &[], &[], &[]),
            stage_step13_04_nft: false,
            stage_step13_05_legb: false,
            stage_normal_gate_native: false,
        },
    )?;
    let control_plane = run_cgroup_binary(
        kit,
        "overdrive-control-plane",
        "integration",
        "cgroup-control-plane-integration",
        15,
        CgroupRunSupport {
            manifests: (
                CGROUP_CONTROL_PLANE,
                STEP13_04_CONTROL_PLANE,
                &[],
                &[],
                STEP13_05_CONTROL_PLANE,
                &[],
                &[],
                CONTROL_PLANE_NORMAL_NATIVE,
            ),
            stage_step13_04_nft: true,
            stage_step13_05_legb: false,
            stage_normal_gate_native: true,
        },
    )?;
    let worker = run_cgroup_binary(
        kit,
        "overdrive-worker",
        "integration",
        "cgroup-worker-integration",
        6,
        CgroupRunSupport {
            manifests: (
                CGROUP_WORKER,
                STEP13_04_WORKER,
                STEP13_04_HOST_LOCAL_FIREWALL,
                STEP13_04_REMOTE_FIREWALL,
                STEP13_05_WORKER,
                STEP13_06_WORKER,
                STEP13_06_ROUTED_WORKER,
                &[],
            ),
            stage_step13_04_nft: true,
            stage_step13_05_legb: true,
            stage_normal_gate_native: false,
        },
    )?;
    let baseline_passed = acceptance.0 + control_plane.0 + worker.0;
    if baseline_passed != CGROUP_DEFAULT_BASELINE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default baseline selected {baseline_passed} passing bodies, expected exactly {CGROUP_DEFAULT_BASELINE_BODIES}"
            ),
            source: std::io::Error::other("the approved cgroup-default manifest changed"),
        });
    }
    let step13_04_passed = acceptance.1 + control_plane.1 + worker.1;
    if step13_04_passed != STEP13_04_NATIVE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default STEP 13-04 extension selected {step13_04_passed} passing bodies, expected exactly {STEP13_04_NATIVE_BODIES}"
            ),
            source: std::io::Error::other("the STEP 13-04 native manifest changed"),
        });
    }
    let step13_05_passed = acceptance.2 + control_plane.2 + worker.2;
    if step13_05_passed != STEP13_05_NATIVE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default STEP 13-05 extension selected {step13_05_passed} passing bodies, expected exactly {STEP13_05_NATIVE_BODIES}"
            ),
            source: std::io::Error::other("the STEP 13-05 native manifest changed"),
        });
    }
    let total_passed = baseline_passed + step13_04_passed + step13_05_passed;
    if total_passed != CGROUP_DEFAULT_TOTAL_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default selected {total_passed} passing bodies in total, expected exactly {CGROUP_DEFAULT_TOTAL_BODIES}"
            ),
            source: std::io::Error::other("the combined cgroup-default manifest changed"),
        });
    }
    let normal_gate_native_passed = acceptance.4 + control_plane.4 + worker.4;
    if normal_gate_native_passed != CONTROL_PLANE_NORMAL_NATIVE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default normal-gate native extension selected {normal_gate_native_passed} passing bodies, expected exactly {CONTROL_PLANE_NORMAL_NATIVE_BODIES}"
            ),
            source: std::io::Error::other("the control-plane normal-gate native manifest changed"),
        });
    }
    let retained_passed = total_passed + normal_gate_native_passed;
    if retained_passed != CGROUP_DEFAULT_RETAINED_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default retained {retained_passed} passing bodies through the normal-gate extension, expected exactly {CGROUP_DEFAULT_RETAINED_BODIES}"
            ),
            source: std::io::Error::other("the retained cgroup-default manifest changed"),
        });
    }
    let step13_06_passed = acceptance.3 + control_plane.3 + worker.3;
    if step13_06_passed != STEP13_06_NATIVE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default STEP 13-06 extension selected {step13_06_passed} passing bodies, expected exactly {STEP13_06_NATIVE_BODIES}"
            ),
            source: std::io::Error::other("the STEP 13-06 native manifest changed"),
        });
    }
    let complete_passed = retained_passed + step13_06_passed;
    if complete_passed != CGROUP_DEFAULT_COMPLETE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default selected {complete_passed} passing bodies after the STEP 13-06 extension, expected exactly {CGROUP_DEFAULT_COMPLETE_BODIES}"
            ),
            source: std::io::Error::other("the complete cgroup-default manifest changed"),
        });
    }

    // Slots 14 and 15 belonged to the now-finished baseline guests above.
    // Reusing them sequentially preserves the registered 16-CID band while
    // giving each STEP 14-01 binary a fresh disposable outer guest.
    let step14_01_cli = run_step14_01_binary(
        kit,
        "overdrive-cli",
        "integration",
        "step14-01-cli",
        14,
        STEP14_01_CLI,
    )?;
    if step14_01_cli != STEP14_01_CLI_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default STEP 14-01 CLI extension selected {step14_01_cli} passing bodies, expected exactly {STEP14_01_CLI_BODIES}"
            ),
            source: std::io::Error::other("the STEP 14-01 CLI manifest changed"),
        });
    }
    let step14_01_control_plane = run_step14_01_binary(
        kit,
        "overdrive-control-plane",
        "integration",
        "step14-01-control-plane",
        15,
        STEP14_01_CONTROL_PLANE,
    )?;
    if step14_01_control_plane != STEP14_01_CONTROL_PLANE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default STEP 14-01 control-plane extension selected {step14_01_control_plane} passing bodies, expected exactly {STEP14_01_CONTROL_PLANE_BODIES}"
            ),
            source: std::io::Error::other("the STEP 14-01 control-plane manifest changed"),
        });
    }
    let step14_01_passed = step14_01_control_plane + step14_01_cli;
    if step14_01_passed != STEP14_01_NATIVE_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default STEP 14-01 extension selected {step14_01_passed} passing bodies, expected exactly {STEP14_01_NATIVE_BODIES}"
            ),
            source: std::io::Error::other("the complete STEP 14-01 manifest changed"),
        });
    }
    Ok(())
}

fn run_stress(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let mut guest =
        GuestRun::boot_with_vcpus(kit, tests, "stress", 11, Vec::new(), Vec::new(), 16)?;
    guest.exec(&exact(STRESS), 1, CELL_BOUND)?;
    guest.finish("stress")
}

fn run_nested_boundary(kit: &GuestKit, tests: &InGuestTests) -> Result<(), GuestKitError> {
    let staging = default_staging_root();
    let modules = WorkDir::create(&staging, "fwd-suite-nested-modules")?;
    let mut extra = base_tools();
    extra.extend(
        [
            "losetup",
            "chmod",
            "mkdir",
            "mkfs.ext4",
            "mknod",
            "mount",
            "prlimit",
            "setpriv",
            "stat",
            "true",
            "uname",
        ]
        .into_iter()
        .map(tool),
    );
    let reflink = make_reflink_volume(modules.path())?;
    extra.push(GuestFile {
        host: reflink,
        guest: PathBuf::from(NESTED_REFLINK_IMAGE),
        with_shared_libraries: false,
    });
    extra.push(GuestFile {
        host: kit.init_bin.clone(),
        guest: PathBuf::from(NESTED_PREBUILT_INIT),
        with_shared_libraries: false,
    });
    extra.push(GuestFile {
        host: kit.kernel_path.clone(),
        guest: PathBuf::from(format!("/boot/vmlinuz-{}", kit.kernel_release)),
        with_shared_libraries: false,
    });
    let xfs = host_module_file(
        kit,
        modules.path(),
        "fs/xfs/xfs",
        Path::new("/lib/overdrive-testing/modules/xfs.ko"),
    )?;
    extra.push(xfs);
    let mut module_paths = Vec::new();
    for &(relative, guest_path) in NESTED_DEVICE_MODULES {
        extra.push(host_module_file(kit, modules.path(), relative, Path::new(guest_path))?);
        module_paths.push(guest_path.to_owned());
    }
    extra.push(GuestFile {
        host: kit.cloud_hypervisor_bin.clone(),
        guest: PathBuf::from("/usr/bin/cloud-hypervisor"),
        with_shared_libraries: true,
    });
    extra.push(tool("systemd-detect-virt"));
    let mut guest = GuestRun::boot(kit, tests, "nested-boundary", 12, extra, module_paths)?;
    guest.command(&["/usr/bin/mkdir", "-p", "/srv/vm"])?;
    guest.command_with_modules(
        &["/lib/overdrive-testing/modules/xfs.ko".to_owned()],
        "/usr/bin/losetup",
        &["/dev/loop0", NESTED_REFLINK_IMAGE],
    )?;
    guest.command(&["/usr/bin/mount", "-t", "xfs", "/dev/loop0", "/srv/vm"])?;
    guest.command(&["/usr/bin/cp", GUEST_PEER_GUEST_PATH, "/srv/vm/reflink-probe"])?;
    guest.command(&[
        "/usr/bin/cp",
        "--reflink=always",
        "/srv/vm/reflink-probe",
        "/srv/vm/reflink-probe.clone",
    ])?;
    let nested_modules = std::mem::take(&mut guest.module_paths);
    guest.command_with_modules(&nested_modules, "/usr/bin/true", &[])?;
    // A minimal devtmpfs has no udev rules, so both devices appear 0600.
    // Reproduce the qualified host's 0660 owner/group access: the production
    // confined identity keeps uid/gid 4200 and the actual device-owner group
    // (gid 0 in this disposable guest) remains its supplementary group.
    guest.command(&["/usr/bin/chmod", "0660", "/dev/kvm", "/dev/vhost-vsock"])?;
    guest.command(&[
        "/usr/bin/stat",
        "-c",
        "NESTED-DEVICE %n uid=%u gid=%g mode=%a",
        "/dev/kvm",
        "/dev/vhost-vsock",
    ])?;
    guest.exec(
        &[TEST_THREADS, NO_CAPTURE, "integration::guest_boundary::"],
        7,
        Duration::from_secs(600),
    )?;
    guest.finish("nested-boundary")
}

fn main() {
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).expect("the prepared GuestKit provisions on metal");
    let tests = InGuestTests::prepared("overdrive-fwd", "integration")
        .expect("cargo xtask guest-kit-prepare staged the integration binary");
    let selected = std::env::var("OVERDRIVE_FWD_GUEST_GROUP").unwrap_or_else(|_| "all".to_owned());
    let runs = [
        ("probe", run_probe_isolation as fn(&GuestKit, &InGuestTests) -> _),
        ("core", run_core),
        ("unload", run_unload),
        ("ulp", run_ulp),
        ("mtls-relay", run_mtls_relay),
        ("cgroup-default", run_cgroup_default),
        ("stress", run_stress),
        ("nested-boundary", run_nested_boundary),
    ];
    let mut matched = false;
    for (name, run) in runs {
        if selected == "all" || selected == name {
            matched = true;
            run(&kit, &tests)
                .unwrap_or_else(|error| panic!("disposable guest group {name}: {error:?}"));
        }
    }
    assert!(matched, "unknown OVERDRIVE_FWD_GUEST_GROUP={selected:?}");
}
