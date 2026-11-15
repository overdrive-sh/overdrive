//! Guest-placed test execution.
//!
//! Boots a fresh disposable `GuestKit` Cloud Hypervisor guest of a declared
//! group, copies the test binary and the group's files in, runs the one
//! selected test inside it and reports the result. The physical Metal kernel
//! is launch substrate, never the module under test. Test-support code only;
//! never shipped by a production crate.
//!
//! Prepare the guest kit on the qualified Metal launcher first:
//!
//! ```text
//! cargo xtask metal run -- cargo xtask guest-kit-prepare
//! ```
//!
//! # Per-test entry points
//!
//! [`inside_guest`] and `placement::run_in_guest` are the two entry points the
//! `#[overdrive_test(guest = "<group>")]` attribute expands to; the attribute
//! never inserts `#[test]`. The body runs in place where [`inside_guest`]
//! holds or `placement::in_metal_run` does not (a machine that is private to
//! the process). On a Metal run, outside a guest, the expansion hands the test
//! binary to the typed `placement::run_in_guest`, which boots one fresh guest
//! of the declared group (through `run_isolated_guest` for an isolated
//! placement), runs that one test inside it and reports the result; the body
//! never runs on the host.

#![cfg(target_os = "linux")]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the test runner reports guest evidence and exact typed failures"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::guest_app::{GUEST_PEER_GUEST_PATH, subcommand};
use crate::guest_kit::cid_bands::{FWD_GUEST_BOUNDARY, INIT_IN_GUEST_LANE};
use crate::guest_kit::{
    DUMMY_MODULE_GUEST_PATH, GuestFile, GuestImageSpec, GuestInit, GuestKit, GuestKitError,
    LOOPBACK_MODULE_GUEST_PATH, ScriptedBootExtra, WorkDir, host_module_file,
};
use crate::guest_lane::{
    INIT_GUEST_LANE_PLACEMENTS, InGuestTests, lane_attempt, run_guest_lane_owner_script,
    run_in_guest_cell_attempt, run_in_guest_owner_cell_attempt,
};
use crate::placement::LanePlacement;
use crate::placement::groups::QUINT_PROFILE;
use crate::vm_fixture::{cargo_target_dir, default_staging_root, workspace_root as checkout_root};
use overdrive_core::vm::config::MemoryPlan;

/// Environment variable the guest attempt sets in the test process it runs
/// inside a guest; [`inside_guest`] reads it.
const IN_GUEST_ENV: &str = "OVERDRIVE_TEST_IN_GUEST";

/// Why a test body did not pass inside its guest.
#[derive(Debug, thiserror::Error)]
pub enum RunInGuestError {
    /// The group's guest context is not prepared on this host, or the group
    /// is unknown. Off a Metal run the placement expansion runs the body in
    /// place and never reaches the guest hand-off, so this is only reported on
    /// a Metal run.
    #[error("guest group {group:?} is not prepared for {test}: {detail}")]
    LaneNotPrepared {
        /// The declared guest group.
        group: String,
        /// The test the guest was started for.
        test: String,
        /// The stage that refused and why.
        detail: String,
    },
    /// The guest could not boot, did not return within its bound, or its
    /// attempt could not be driven to completion.
    #[error("guest group {group:?} could not run {test}: {detail}")]
    GuestExecution {
        /// The declared guest group.
        group: String,
        /// The test the guest was started for.
        test: String,
        /// The stage that failed and why.
        detail: String,
    },
    /// The test ran inside the guest and its process exited unsuccessfully.
    /// The guest console, which carries the failure output, is printed before
    /// this error is returned.
    #[error("{test} failed inside the {group:?} guest with exit status {status}")]
    BodyFailed {
        /// The declared guest group.
        group: String,
        /// The failed test.
        test: String,
        /// The test process's exit status inside the guest.
        status: i32,
    },
}

/// Whether the current process is the test process of a guest attempt.
///
/// True only in the process `placement::run_in_guest` starts inside a guest; the host-side half of a guest-placed test sees `false`.
#[must_use]
pub fn inside_guest() -> bool {
    std::env::var_os(IN_GUEST_ENV).is_some_and(|value| value == "1")
}

/// Runs `test_name` of the test binary `exe` inside one fresh guest of
/// `group` and reports the outcome; the isolated-placement path of
/// `placement::run_in_guest`.
///
/// `test_name` is the full libtest name (module path without the crate
/// segment). One guest is booted per call. Boot and total wall-clock
/// durations are written to stderr as an `OVERDRIVE_GUEST_KIT_TIMING` line.
///
/// # Preconditions
///
/// The guest kit is prepared (`cargo xtask guest-kit-prepare`) and the caller
/// has found `placement::in_metal_run()` true and [`inside_guest`] false.
///
/// # Errors
///
/// [`RunInGuestError::LaneNotPrepared`] for an unprepared host or an unknown
/// group (named in the detail), [`RunInGuestError::GuestExecution`] when the
/// guest cannot boot or does not return, [`RunInGuestError::BodyFailed`] when
/// the test exits unsuccessfully inside the guest.
pub(crate) fn run_isolated_guest(
    group: &str,
    profile: &str,
    exe: &Path,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    let program = exe.to_string_lossy().into_owned();
    // The host half runs only because nextest selected this test, so an
    // `#[ignore]`d body it selected (`--run-ignored`) must run in the guest too.
    let args = [
        "--exact".to_owned(),
        test_name.to_owned(),
        NO_CAPTURE.to_owned(),
        "--include-ignored".to_owned(),
    ];
    let binary_id = std::env::var("NEXTEST_BINARY_ID").unwrap_or_default();
    let named = |detail: String| (group.to_owned(), test_name.to_owned(), detail);
    let attempt =
        GroupAttempt { group, profile, program: &program, args: &args, binary_id: &binary_id };
    match run_nextest_group_attempt(&attempt, test_name) {
        Ok(0) => Ok(()),
        Ok(status) => Err(RunInGuestError::BodyFailed {
            group: group.to_owned(),
            test: test_name.to_owned(),
            status,
        }),
        Err(NextestDispatchFailure::LaneNotPrepared(detail)) => {
            let (group, test, detail) = named(detail);
            Err(RunInGuestError::LaneNotPrepared { group, test, detail })
        }
        Err(NextestDispatchFailure::GuestExecution(detail)) => {
            let (group, test, detail) = named(detail);
            Err(RunInGuestError::GuestExecution { group, test, detail })
        }
    }
}

const STANDARD_OUTER_GUEST_MIB: u32 = 4_096;
const QUINT_SPECS_DIR: &str = "specs/quint";
const QUINT_RUNTIME_DIR: &str = "guest-kit/quint-test-runtime";
const QUINT_RUNTIME_FILES_DIR: &str = "files";
const QUINT_RUNTIME_MANIFEST: &str = "manifest.v1";
const QUINT_RUNTIME_FORMAT: &str = "overdrive-quint-test-runtime-v1";
const NO_CAPTURE: &str = "--nocapture";
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
/// Host tools every `cgroup-default` guest stages, batched or per-test. The
/// guests boot `serve`, whose shared mTLS nft program is installed
/// unconditionally, and the bodies that inspect it shell out to `nft`; this is
/// the single definition, alongside the module closure below.
///
/// Beyond the boot and nft tooling, bodies across the group shell out to `ss`
/// (socket-state oracles), `strace` and `uname` (syscall evidence), `ln` (the
/// `/var/run` compatibility link), `openssl` (chain verification of minted
/// certificates) and `prlimit` (the VMM launch wrapper the startup probe execs
/// under the launch seccomp filter). Staging them for the whole group, not per selected cell, is
/// what keeps a newly placed body from failing with "command not found".
const CGROUP_DEFAULT_TOOLS: [&str; 14] = [
    "cat", "chmod", "ip", "ln", "mkdir", "mount", "nft", "openssl", "prlimit", "sh", "ss", "stat",
    "strace", "uname",
];
/// `ip netns add` publishes handles under `/run/netns`, while the authored
/// fixtures and production's `overdrive-netlink` open the traditional
/// `/var/run/netns` spelling. The standard `/var/run -> /run` link makes both
/// resolve to one directory in the disposable guest.
const NETNS_VAR_RUN_COMPAT: &str = "set -eu; mkdir -p /var /run/netns; test ! -e /var/run; test ! -L /var/run; ln -s /run /var/run; test -L /var/run; test \"$(cd /var/run && pwd -P)\" = /run; test -d /var/run/netns";
/// Every `cgroup-default` guest boots `serve`, whose always-installed shared
/// mTLS nft program needs the `nf_tables` core, veth, TLS, and the complete
/// tproxy / reject / fib expression closure. The guest has no `/lib/modules`,
/// so nothing autoloads: this is the single definition both the batched and
/// the per-test attempts stage, unconditionally for the group.
fn cgroup_default_module_specs() -> Vec<(&'static str, &'static str)> {
    let mut specs = NESTED_DEVICE_MODULES.to_vec();
    specs.extend([
        ("net/netfilter/nfnetlink", NFNETLINK_MODULE_GUEST_PATH),
        ("net/netfilter/nf_tables", NF_TABLES_MODULE_GUEST_PATH),
        ("drivers/net/veth", VETH_MODULE_GUEST_PATH),
        ("net/tls/tls", TLS_MODULE_GUEST_PATH),
    ]);
    specs.extend_from_slice(STEP13_04_NFT_MODULES);
    specs.extend_from_slice(STEP13_06_NFT_MODULES);
    specs
}
const NESTED_GUEST_KIT_MODULE_SOURCES: &[&str] = &[
    "drivers/net/dummy",
    "net/sched/sch_prio",
    "net/sched/sch_netem",
    "net/ipv4/inet_diag",
    "net/ipv4/tcp_diag",
    "net/ipv4/udp_diag",
    "net/vmw_vsock/vsock",
    "net/vmw_vsock/vmw_vsock_virtio_transport_common",
    "net/vmw_vsock/vmw_vsock_virtio_transport",
    "net/vmw_vsock/vsock_loopback",
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
// The current Nextest profiles cap an attempt below this value; Nextest owns
// timeout, retry, and aggregate-status decisions for the selected attempt.
const NEXTTEST_ATTEMPT_RESPONSE_BOUND: Duration = Duration::from_secs(86_400);

// The two existing correctness bodies declare exactly 64 workload VMs at
// 128 MiB each. Account for every inner VMM at the production MemoryPlan
// cgroup ceiling, then retain the established 4 GiB outer-guest envelope for
// the node kernel, serve composition, test process and fixture tools. This is
// static capacity configuration, not a density or memory measurement.
const BYTES_PER_MIB: u64 = 1024 * 1024;
const VM_COHORT_WORKLOADS: u64 = 64;
const VM_COHORT_WORKLOAD_MEMORY_BYTES: u64 = 128 * BYTES_PER_MIB;
const VM_COHORT_REQUIRED_OUTER_BYTES: u64 = MemoryPlan::derive(VM_COHORT_WORKLOAD_MEMORY_BYTES)
    .cgroup_max_bytes()
    .saturating_mul(VM_COHORT_WORKLOADS)
    .saturating_add((STANDARD_OUTER_GUEST_MIB as u64) * BYTES_PER_MIB);
const VM_COHORT_OUTER_GUEST_MIB: u32 = 12_821;
const _: () =
    assert!((VM_COHORT_OUTER_GUEST_MIB as u64) * BYTES_PER_MIB >= VM_COHORT_REQUIRED_OUTER_BYTES);
const _: () = assert!(
    ((VM_COHORT_OUTER_GUEST_MIB - 1) as u64) * BYTES_PER_MIB < VM_COHORT_REQUIRED_OUTER_BYTES
);
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
/// The one group whose guests run `serve`, which requires the raised
/// `RLIMIT_NOFILE` budget. SSOT for both the batched and per-test paths.
const CGROUP_DEFAULT_GROUP: &str = "cgroup-default";
const DECLARED_LISTENER_TCP_CAPTURE_TEST: &str = "integration::inbound_rules_only_for_tcp_declared_listeners::a_udp_declared_listener_gets_no_inbound_capture_and_a_tcp_one_does";
/// Argv prefix that raises `RLIMIT_NOFILE` and then execs the remaining argv.
const fn cgroup_nofile_prefix() -> [&'static str; 4] {
    ["/usr/bin/sh", "-c", CGROUP_NOFILE_EXEC, CGROUP_DEFAULT_GROUP]
}
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
test -r "$1"
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
bpf_sha256="$(/usr/bin/python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$1")"
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

// The XDP_FIB_HIT cells resolve their client/backend addresses through a
// test-owned veth next hop with a permanent neighbour entry, and need IPv4
// forwarding because `bpf_fib_lookup` refuses a lookup from an ingress device
// without it (`bpf_ipv4_fib_lookup`, net/core/filter.c @ v7.0). 198.18.0.0/24
// (RFC 2544) is disjoint from every address the cells or the guest use.
const XDP_FIB_HIT_SETUP: &str = r#"
set -eu
test "$(/usr/bin/cat /proc/sys/net/ipv4/ip_forward)" = 0
if /usr/bin/ip -4 route get 10.0.0.100 >/dev/null 2>&1; then
  echo "XDP-FIB-HIT route precondition unexpectedly already resolves 10.0.0.100" >&2
  exit 1
fi
/usr/bin/ip link add ovd-gk-fib type veth peer name ovd-gk-fibp
/usr/bin/ip link set ovd-gk-fib address 02:00:5e:12:00:01
/usr/bin/ip addr add 198.18.0.1/24 dev ovd-gk-fib
/usr/bin/ip link set ovd-gk-fibp up
/usr/bin/ip link set ovd-gk-fib up
/usr/bin/ip neigh replace 198.18.0.2 lladdr 02:00:5e:12:00:02 dev ovd-gk-fib nud permanent
/usr/bin/ip route add default via 198.18.0.2 dev ovd-gk-fib metric 42762
echo 1 > /proc/sys/net/ipv4/ip_forward
for destination in 10.0.0.100 10.1.0.5; do
  route="$(/usr/bin/ip -4 route get "$destination")"
  case "$route" in
    *"via 198.18.0.2 dev ovd-gk-fib"*) ;;
    *) echo "XDP-FIB-HIT route precondition mismatch: $route" >&2; exit 1 ;;
  esac
done
echo "XDP-FIB-HIT precondition readback: $(/usr/bin/ip neigh show 198.18.0.2 dev ovd-gk-fib) forwarding=$(/usr/bin/cat /proc/sys/net/ipv4/ip_forward)"
"#;
const XDP_FIB_HIT_CLEANUP: &str = r#"
set -eu
echo 0 > /proc/sys/net/ipv4/ip_forward
/usr/bin/ip link del ovd-gk-fib
if /usr/bin/ip -4 route get 10.0.0.100 >/dev/null 2>&1; then
  echo "XDP-FIB-HIT test-owned route survived cleanup" >&2
  exit 1
fi
echo "XDP-FIB-HIT cleanup readback: unroutable forwarding=$(/usr/bin/cat /proc/sys/net/ipv4/ip_forward)"
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

#[derive(Clone, Copy)]
struct NestedVmGuestResources {
    memory_mib: u32,
    vcpus: u8,
}

impl NestedVmGuestResources {
    const STANDARD: Self = Self { memory_mib: STANDARD_OUTER_GUEST_MIB, vcpus: 8 };

    const VM_COHORT: Self = Self { memory_mib: VM_COHORT_OUTER_GUEST_MIB, vcpus: 8 };
}

struct GuestRun {
    guest: crate::guest_kit::ScriptedGuest,
    module_paths: Vec<String>,
    _work: WorkDir,
}

impl GuestRun {
    fn boot_with_files(
        kit: &GuestKit,
        name: &str,
        slot: u32,
        extra: Vec<GuestFile>,
        module_paths: Vec<String>,
        memory_mib: u32,
        vcpus: u8,
    ) -> Result<Self, GuestKitError> {
        let staging = default_staging_root();
        let work = WorkDir::create(&staging, &format!("fwd-suite-{name}"))?;
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
                memory_mib: Some(memory_mib),
                vcpus: Some(vcpus),
            },
        )?;
        Ok(Self { guest, module_paths, _work: work })
    }

    fn exec_nextest_attempt(
        &mut self,
        program: &str,
        test_args: &[String],
        environment: &[(String, String)],
        working_directory: &Path,
        raise_nofile: bool,
        bound: Duration,
    ) -> Result<i32, GuestKitError> {
        let command = nextest_attempt_command(
            &self.module_paths,
            program,
            test_args,
            environment,
            working_directory,
            raise_nofile,
        );
        let argv = command.iter().map(String::as_str).collect::<Vec<_>>();
        self.guest.peer().exec(&argv, bound).map_err(|source| GuestKitError::Staging {
            detail: format!("selected nextest attempt {program:?} did not return within {bound:?}"),
            source: std::io::Error::other(format!(
                "{source}; full disposable guest console:\n{}",
                self.guest.vm.console()
            )),
        })
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

fn workspace_bpf_guest_path() -> PathBuf {
    // The test ELF embeds this checkout's CARGO_MANIFEST_DIR; mirror its
    // source-owned target path instead of assuming a singleton ~/overdrive.
    workspace_root().join("target/bpf/overdrive_bpf.o")
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
        // Match forwarder_module::built_module_path in this checkout's test
        // ELF even when the Metal source lives under its identity directory.
        guest: workspace_root().join("target/kmod").join(&kit.kernel_release).join("ovd_fwd.ko"),
        with_shared_libraries: false,
    }
}

fn setup_dataplane_native_guest(guest: &mut GuestRun) -> Result<(), GuestKitError> {
    let bpf_object = workspace_bpf_guest_path();
    guest.command(&[
        "/usr/bin/sh",
        "-c",
        DATAPLANE_NATIVE_SETUP,
        "dataplane-native",
        bpf_object.to_str().expect("workspace BPF guest path is UTF-8"),
    ])
}

fn cgroup_tools(
    kit: &GuestKit,
    support_dir: &Path,
    stage_step13_05_legb: bool,
    stage_normal_gate_native: bool,
) -> Result<Vec<GuestFile>, GuestKitError> {
    let mut files: Vec<_> = CGROUP_DEFAULT_TOOLS.into_iter().map(tool).collect();
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
                    "the qualified kernel ships no STEP 14 nested GuestKit module source {relative}"
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

/// Stage the exact outer-guest substrate the nested production-serve
/// integration binaries consume. This is the existing
/// GuestKit-inside-GuestKit shape from `nested-boundary`, plus the production
/// serve dependencies those binaries reach: the BPF object, netlink/nft
/// modules, veth, kTLS and the ordinary host tools. It adds no test hook or
/// alternate production composition.
fn nested_vm_test_support(
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
    // The nested scripted-image builder asks its existing `ldd`-backed
    // `GuestKit::image` path to inspect `tc`. `ldd` is a shell script on the
    // qualified Metal host, so stage it without recursively asking ldd for
    // ldd's libraries, and stage its Bash interpreter at the shebang path.
    let mut ldd = tool("ldd");
    ldd.with_shared_libraries = false;
    extra.push(ldd);
    let mut bash = tool("bash");
    bash.guest = PathBuf::from("/bin/bash");
    extra.push(bash);
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
        detail: format!("creating STEP 14 guest resolver target at {}", resolver.display()),
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
    for (relative, guest_path) in cgroup_default_module_specs() {
        stage_module(relative, guest_path)?;
    }
    Ok((extra, module_paths))
}

/// The `PATH` of a `NESTED_GUEST_LAUNCH` attempt. These bodies resolve
/// `cloud-hypervisor`, `which` and the image tools through `PATH` as on the
/// metal host, where the VMM binary sits alone in its own directory; one body
/// removes that directory from `PATH` and expects only the VMM to go missing.
const NESTED_GUEST_LAUNCH_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
const NESTED_GUEST_LAUNCH_NODE_SETUP: &str = r#"
set -eu
mkdir -p /srv/vm/overdrive-testing
chmod 0711 /srv/vm/overdrive-testing
# The image is staged under the runner's restrictive umask; the metal host's
# system directories are world-readable, and a body that drops to the confined
# VMM uid executes tools and loads libraries from them.
for directory in /usr /lib /lib64 /bin /etc; do
  if [ -d "$directory" ]; then chmod -R go+rX "$directory"; fi
done
for directory in /sys/fs/cgroup/overdrive.slice /sys/fs/cgroup/overdrive.slice/workloads.slice; do
  mkdir -p "$directory"
  printf '%s\n' '+cpu +memory +io +pids' > "$directory/cgroup.subtree_control"
done
"#;
/// The metal host's VMM directory, mirrored in the outer guest.
const NESTED_GUEST_LAUNCH_VMM_PATH: &str = "/usr/local/bin/cloud-hypervisor";
/// A root-only account database: the outer guest ships no `/etc/passwd`, and
/// one body asserts the production VMM uid has no entry in it.
const NESTED_GUEST_LAUNCH_PASSWD: &str = "root:x:0:0:root:/root:/usr/bin/sh\n";

/// Stage what the bodies in `NESTED_GUEST_LAUNCH` resolve through the outer
/// guest's own filesystem on top of `nested_vm_test_support`: the VMM binary in
/// the directory the metal host keeps it in, `which`, `umount`, a static
/// `busybox`, a `/bin/sh` for `which`'s interpreter line, and `/etc/passwd`. A
/// lane driver (`IN_GUEST_LANE_DRIVERS`) also finds the prepared in-guest cell
/// binary at its workspace path, as it does on the host.
fn nested_guest_launch_support(
    dir: &Path,
    environment: AttemptEnvironment,
    extra: &mut Vec<GuestFile>,
) -> Result<(), GuestKitError> {
    let vmm = extra
        .iter_mut()
        .find(|file| file.guest == Path::new("/usr/bin/cloud-hypervisor"))
        .ok_or_else(|| GuestKitError::Staging {
            detail: "the nested guest support stages no cloud-hypervisor".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        })?;
    vmm.guest = PathBuf::from(NESTED_GUEST_LAUNCH_VMM_PATH);
    // `which` is a `#!/bin/sh` script: no shared libraries of its own.
    let mut which = tool("which");
    which.with_shared_libraries = false;
    extra.push(which);
    extra.push(tool("umount"));
    let mut busybox = tool("busybox");
    busybox.with_shared_libraries = false;
    extra.push(busybox);
    extra.push(GuestFile { guest: PathBuf::from("/bin/sh"), ..tool("sh") });
    let passwd = dir.join("passwd");
    fs::write(&passwd, NESTED_GUEST_LAUNCH_PASSWD).map_err(|source| GuestKitError::Staging {
        detail: format!("creating guest account database at {}", passwd.display()),
        source,
    })?;
    extra.push(GuestFile {
        host: passwd,
        guest: PathBuf::from("/etc/passwd"),
        with_shared_libraries: false,
    });
    if environment == AttemptEnvironment::NestedLaunchRustc {
        nested_guest_build_support(extra)?;
        stage_prepare_overdrive_init(extra)?;
    }
    if environment == AttemptEnvironment::NestedLaunchLane {
        let cells = InGuestTests::prepared("overdrive-init", "overdrive-init")?;
        extra.push(GuestFile {
            host: cells.binary.clone(),
            guest: cells.binary,
            with_shared_libraries: false,
        });
    }
    Ok(())
}

/// `rustc --print sysroot` on the runner: the toolchain the metal host's
/// bodies compile with.
fn rust_sysroot() -> Result<PathBuf, GuestKitError> {
    let output = Command::new("rustc").args(["--print", "sysroot"]).output().map_err(|source| {
        GuestKitError::Staging {
            detail: "locating the runner's Rust toolchain for the outer guest".to_owned(),
            source,
        }
    })?;
    if !output.status.success() {
        return Err(GuestKitError::Staging {
            detail: format!("rustc --print sysroot exited {:?}", output.status.code()),
            source: std::io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()),
        });
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

/// Every regular file below `dir`, staged at its own path.
fn stage_tree(dir: &Path, files: &mut Vec<GuestFile>) -> Result<(), GuestKitError> {
    let staging = |source: std::io::Error| GuestKitError::Staging {
        detail: format!("listing {} for the outer guest", dir.display()),
        source,
    };
    for entry in fs::read_dir(dir).map_err(staging)? {
        let path = entry.map_err(staging)?.path();
        if path.is_dir() {
            stage_tree(&path, files)?;
        } else {
            files.push(GuestFile { host: path.clone(), guest: path, with_shared_libraries: false });
        }
    }
    Ok(())
}

/// The static musl `overdrive-init` at the workspace path
/// `examples/public-ingress-gateway/prepare.sh` reads it from
/// (`target/x86_64-unknown-linux-musl/release/overdrive-init`). The guest kit's
/// own init is already staged at `NESTED_PREBUILT_INIT`; the same binary is
/// staged a second time at the path the script resolves.
fn stage_prepare_overdrive_init(extra: &mut Vec<GuestFile>) -> Result<(), GuestKitError> {
    let init = extra
        .iter()
        .find(|file| file.guest == Path::new(NESTED_PREBUILT_INIT))
        .map(|file| file.host.clone())
        .ok_or_else(|| GuestKitError::Staging {
            detail: "the nested guest support stages no prebuilt overdrive-init".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        })?;
    extra.push(GuestFile {
        host: init,
        guest: workspace_root().join("target/x86_64-unknown-linux-musl/release/overdrive-init"),
        with_shared_libraries: false,
    });
    Ok(())
}

/// What the bodies in `NESTED_GUEST_RUSTC` resolve through the outer guest's
/// own filesystem on top of `nested_guest_launch_support`: the runner's
/// `rustc` at its own sysroot path with its musl target libraries, the `cc`
/// linker driver with `collect2`, the LTO plugin and `ld`, the tools
/// `examples/public-ingress-gateway/prepare.sh` and the gateway kernel oracles
/// invoke, and the checked-in sources those bodies read from the checkout.
fn nested_guest_build_support(extra: &mut Vec<GuestFile>) -> Result<(), GuestKitError> {
    let sysroot = rust_sysroot()?;
    extra.push(GuestFile {
        host: sysroot.join("bin/rustc"),
        guest: sysroot.join("bin/rustc"),
        with_shared_libraries: true,
    });
    stage_tree(&sysroot.join("lib/rustlib/x86_64-unknown-linux-musl/lib"), extra)?;

    let driver = fs::canonicalize("/usr/bin/cc").map_err(|source| GuestKitError::Staging {
        detail: "resolving the runner's cc for the outer guest".to_owned(),
        source,
    })?;
    extra.push(GuestFile {
        host: driver,
        guest: PathBuf::from("/usr/bin/cc"),
        with_shared_libraries: true,
    });
    let collect2 =
        Command::new("cc").arg("-print-prog-name=collect2").output().map_err(|source| {
            GuestKitError::Staging { detail: "asking cc for collect2".to_owned(), source }
        })?;
    let collect2 = PathBuf::from(String::from_utf8_lossy(&collect2.stdout).trim());
    let libexec =
        collect2.parent().map(Path::to_path_buf).ok_or_else(|| GuestKitError::Staging {
            detail: format!("cc reported collect2 at {}", collect2.display()),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        })?;
    extra.push(GuestFile { host: collect2.clone(), guest: collect2, with_shared_libraries: true });
    let plugin = libexec.join("liblto_plugin.so");
    extra.push(GuestFile { host: plugin.clone(), guest: plugin, with_shared_libraries: false });
    extra.push(tool("ld"));

    for name in [
        "awk",
        "bpftool",
        "chroot",
        "cmp",
        "cut",
        "depmod",
        "dirname",
        "file",
        "grep",
        "head",
        "id",
        "install",
        "modinfo",
        "mountpoint",
        "readelf",
        "rm",
        "sync",
        "tr",
        "test",
        "timeout",
        "wc",
    ] {
        extra.push(tool(name));
    }
    let magic = PathBuf::from("/usr/share/misc/magic.mgc");
    extra.push(GuestFile { host: magic.clone(), guest: magic, with_shared_libraries: false });

    let root = workspace_root();
    for relative in ["examples/public-ingress-gateway", "tests/conformance/guest"] {
        let mut sources = Vec::new();
        stage_tree(&root.join(relative), &mut sources)?;
        extra.extend(sources);
    }
    Ok(())
}

#[derive(Debug)]
enum NextestDispatchFailure {
    LaneNotPrepared(String),
    GuestExecution(String),
}

/// The environment a per-test attempt stages and sets up inside its group:
/// what the closed registry's profile of the same name selects.
///
/// The declared profile selects it ([`Self::of_profile`]). Everything after the
/// choice reads this value, never the test name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptEnvironment {
    /// The group's base environment (also `cgroup-default`'s `quint`, whose
    /// extra inputs are staged from the profile name alone).
    Base,
    /// `mtls-relay` `native`: the dataplane's native modules and native setup.
    MtlsNative,
    /// `mtls-relay` `relay`: only the tls module is loaded.
    MtlsRelay,
    /// `mtls-relay` `fib-hit`: `native` plus the test-owned veth next hop,
    /// route and IPv4 forwarding the `bpf_fib_lookup` success cells need.
    MtlsFibHit,
    /// `cgroup-default` `dummy-link`: `base` plus the dummy module.
    DummyLink,
    /// `cgroup-default` `host-route`: `dummy-link` plus the host-local route.
    HostRoute,
    /// `cgroup-default` `legb-tools`: `base` plus sysctl.
    LegbTools,
    /// `cgroup-default` `legb-audit`: `legb-tools` plus the LEGB setup and audit.
    LegbAudit,
    /// `cgroup-default` `routed-route`: `base` plus the routed-route lifecycle.
    RoutedRoute,
    /// `cgroup-default` `normal-gate`: `base` plus the control-plane native setup.
    NormalGate,
    /// `cgroup-default` `nested`: the nested-KVM substrate.
    Nested,
    /// `cgroup-default` `nested-cohort`: `nested` with the cohort's memory.
    NestedCohort,
    /// `cgroup-default` `nested-launch`: `nested` plus the launched VMM.
    NestedLaunch,
    /// `cgroup-default` `nested-launch-rustc`: `nested-launch` plus the Rust sysroot.
    NestedLaunchRustc,
    /// `cgroup-default` `nested-launch-lane`: `nested-launch` plus the lane test binary.
    NestedLaunchLane,
}

impl AttemptEnvironment {
    /// The environment a declared profile of `group` selects. An unknown
    /// profile of a known group cannot reach here (the attribute is checked at
    /// build time); one that does is an unprepared kit, reported by name.
    fn of_profile(group: &str, profile: &str) -> Result<Self, NextestDispatchFailure> {
        let environment = match (group, profile) {
            ("mtls-relay", "native") => Self::MtlsNative,
            ("mtls-relay", "relay") => Self::MtlsRelay,
            ("mtls-relay", "fib-hit") => Self::MtlsFibHit,
            ("cgroup-default", "dummy-link") => Self::DummyLink,
            ("cgroup-default", "host-route") => Self::HostRoute,
            ("cgroup-default", "legb-tools") => Self::LegbTools,
            ("cgroup-default", "legb-audit") => Self::LegbAudit,
            ("cgroup-default", "routed-route") => Self::RoutedRoute,
            ("cgroup-default", "normal-gate") => Self::NormalGate,
            ("cgroup-default", "nested") => Self::Nested,
            ("cgroup-default", "nested-cohort") => Self::NestedCohort,
            ("cgroup-default", "nested-launch") => Self::NestedLaunch,
            ("cgroup-default", "nested-launch-rustc") => Self::NestedLaunchRustc,
            ("cgroup-default", "nested-launch-lane") => Self::NestedLaunchLane,
            ("cgroup-default", "base" | QUINT_PROFILE)
            | ("probe" | "core" | "unload" | "ulp" | "stress" | "nested-boundary", "base") => {
                Self::Base
            }
            _ => {
                return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                    "guest group {group:?} has no profile {profile:?}"
                )));
            }
        };
        Ok(environment)
    }

    /// The dummy module is loaded at boot.
    const fn stages_dummy_module(self) -> bool {
        matches!(self, Self::DummyLink | Self::HostRoute)
    }

    /// sysctl is staged for the LEGB fixtures.
    const fn stages_legb_tools(self) -> bool {
        matches!(self, Self::LegbTools | Self::LegbAudit)
    }

    const fn stages_normal_gate(self) -> bool {
        matches!(self, Self::NormalGate)
    }

    /// The body boots its own guests (the outer guest is the nested substrate
    /// plus the launched VMM).
    const fn launches_guests(self) -> bool {
        matches!(self, Self::NestedLaunch | Self::NestedLaunchRustc | Self::NestedLaunchLane)
    }

    /// The outer guest of a `cgroup-default` profile that needs the nested-KVM
    /// substrate; `None` for the profiles that run in the ordinary plan.
    const fn nested_resources(self) -> Option<NestedVmGuestResources> {
        match self {
            Self::NestedCohort => Some(NestedVmGuestResources::VM_COHORT),
            Self::Nested
            | Self::NestedLaunch
            | Self::NestedLaunchRustc
            | Self::NestedLaunchLane => Some(NestedVmGuestResources::STANDARD),
            _ => None,
        }
    }
}

struct CgroupAttemptFiles {
    extra: Vec<GuestFile>,
    module_paths: Vec<String>,
}

struct MtlsAttemptFiles {
    extra: Vec<GuestFile>,
    module_paths: Vec<String>,
    native_module_paths: Vec<String>,
    setup_native: bool,
}

struct NextestGuestPlan {
    name: &'static str,
    cid: u32,
    extra: Vec<GuestFile>,
    module_paths: Vec<String>,
    memory_mib: u32,
    vcpus: u8,
    environment: AttemptEnvironment,
    /// True for the ordinary (non-nested) `cgroup-default` plan.
    cgroup: bool,
    mtls_native_paths: Vec<String>,
    setup_mtls_native: bool,
    nested: bool,
}

fn nextest_environment() -> Result<Vec<(String, String)>, NextestDispatchFailure> {
    const PROPTEST_KEYS: &[&str] = &[
        "PROPTEST_CASES",
        "PROPTEST_DISABLE_FAILURE_PERSISTENCE",
        "PROPTEST_FORK",
        "PROPTEST_MAX_DEFAULT_SIZE_RANGE",
        "PROPTEST_MAX_FLAT_MAP_REGENS",
        "PROPTEST_MAX_GLOBAL_REJECTS",
        "PROPTEST_MAX_LOCAL_REJECTS",
        "PROPTEST_MAX_SHRINK_ITERS",
        "PROPTEST_MAX_SHRINK_TIME",
        "PROPTEST_RNG_ALGORITHM",
        "PROPTEST_RNG_SEED",
        "PROPTEST_TIMEOUT",
        "PROPTEST_VERBOSE",
    ];
    let mut values = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            let selected = key.starts_with("NEXTEST_")
                || PROPTEST_KEYS.contains(&key.as_str())
                || matches!(key.as_str(), "OVERDRIVE_BPF_NATIVE" | "RUST_BACKTRACE" | "RUST_LOG");
            selected.then_some((key, value))
        })
        .map(|(key, value)| {
            let value = value.into_string().map_err(|_| {
                NextestDispatchFailure::LaneNotPrepared(format!(
                    "the selected environment value for {key} is not valid UTF-8"
                ))
            })?;
            Ok((key, value))
        })
        .collect::<Result<Vec<_>, NextestDispatchFailure>>()?;
    values.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(values)
}

fn nextest_attempt_files(
    program: &str,
    environment: &[(String, String)],
) -> Result<(Vec<GuestFile>, PathBuf), NextestDispatchFailure> {
    let current_dir = std::env::current_dir().map_err(|error| {
        NextestDispatchFailure::GuestExecution(format!(
            "reading the selected attempt working directory: {error}"
        ))
    })?;
    let host_path = PathBuf::from(program);
    let guest_path =
        if host_path.is_absolute() { host_path.clone() } else { current_dir.join(&host_path) };
    let host_binary = host_path
        .canonicalize()
        .or_else(|_| current_dir.join(&host_path).canonicalize())
        .map_err(|error| {
            NextestDispatchFailure::LaneNotPrepared(format!(
                "the selected nextest executable {program:?} is absent: {error}"
            ))
        })?;
    let mut files =
        vec![GuestFile { host: host_binary, guest: guest_path, with_shared_libraries: true }];
    for (key, value) in environment.iter().filter(|(key, _)| key.starts_with("NEXTEST_BIN_EXE_")) {
        let host_path = PathBuf::from(value);
        let guest_path =
            if host_path.is_absolute() { host_path.clone() } else { current_dir.join(&host_path) };
        let host = host_path
            .canonicalize()
            .or_else(|_| current_dir.join(&host_path).canonicalize())
            .map_err(|error| {
                NextestDispatchFailure::LaneNotPrepared(format!(
                    "the selected nextest helper {key} at {value:?} is absent: {error}"
                ))
            })?;
        files.push(GuestFile { host, guest: guest_path, with_shared_libraries: true });
    }
    Ok((files, current_dir))
}

struct PreparedQuintRuntime {
    files: std::collections::BTreeSet<PathBuf>,
    shared_libraries: std::collections::BTreeSet<PathBuf>,
}

struct QuintRuntimeRecords {
    fields: std::collections::BTreeMap<String, String>,
    files: std::collections::BTreeSet<PathBuf>,
    shared_libraries: std::collections::BTreeSet<PathBuf>,
}

fn parse_quint_runtime_records<'a>(
    lines: impl Iterator<Item = &'a str>,
    manifest_path: &Path,
) -> Result<QuintRuntimeRecords, NextestDispatchFailure> {
    let mut fields = std::collections::BTreeMap::new();
    let mut files = std::collections::BTreeSet::new();
    let mut shared_libraries = std::collections::BTreeSet::new();
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                "malformed Quint runtime manifest line in {}",
                manifest_path.display()
            )));
        };
        match key {
            "file" => {
                if !files.insert(PathBuf::from(value)) {
                    return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                        "duplicate Quint runtime file in {}: {value}",
                        manifest_path.display()
                    )));
                }
            }
            "shared" => {
                if !shared_libraries.insert(PathBuf::from(value)) {
                    return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                        "duplicate shared-library input in {}: {value}",
                        manifest_path.display()
                    )));
                }
            }
            "quint_version"
            | "node_version"
            | "rust_evaluator_version"
            | "node_guest_path"
            | "rust_evaluator_guest_path"
            | "launcher_guest_path"
            | "cli_wrapper_guest_path"
            | "cli_entry_guest_path" => {
                if fields.insert(key.to_owned(), value.to_owned()).is_some() {
                    return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                        "duplicate Quint runtime manifest field {key:?}"
                    )));
                }
            }
            _ => {
                return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                    "unknown Quint runtime manifest field {key:?}"
                )));
            }
        }
    }
    Ok(QuintRuntimeRecords { fields, files, shared_libraries })
}

fn normalized_quint_guest_path(
    fields: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> Result<PathBuf, NextestDispatchFailure> {
    let path = fields.get(key).map(PathBuf::from).ok_or_else(|| {
        NextestDispatchFailure::LaneNotPrepared(format!("Quint runtime manifest has no {key}"))
    })?;
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(component, std::path::Component::CurDir | std::path::Component::ParentDir)
        })
    {
        return Err(NextestDispatchFailure::LaneNotPrepared(format!(
            "Quint runtime manifest has a non-normalized {key}: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn prepared_quint_runtime(prepared: &Path) -> Result<PreparedQuintRuntime, NextestDispatchFailure> {
    let manifest_path = prepared.join(QUINT_RUNTIME_MANIFEST);
    let manifest = fs::read_to_string(&manifest_path).map_err(|error| {
        NextestDispatchFailure::LaneNotPrepared(format!(
            "Quint test runtime is not prepared at {}: {error}; run cargo xtask guest-kit-prepare",
            manifest_path.display()
        ))
    })?;
    let mut lines = manifest.lines();
    if lines.next() != Some(QUINT_RUNTIME_FORMAT) {
        return Err(NextestDispatchFailure::LaneNotPrepared(format!(
            "unsupported Quint runtime manifest at {}",
            manifest_path.display()
        )));
    }
    let QuintRuntimeRecords { fields, files, shared_libraries } =
        parse_quint_runtime_records(lines, &manifest_path)?;
    for key in ["quint_version", "node_version", "rust_evaluator_version"] {
        if fields
            .get(key)
            .is_none_or(|value| value.is_empty() || value.chars().any(char::is_control))
        {
            return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                "Quint runtime manifest has no valid {key}"
            )));
        }
    }
    let node_guest_path = normalized_quint_guest_path(&fields, "node_guest_path")?;
    let evaluator_guest_path = normalized_quint_guest_path(&fields, "rust_evaluator_guest_path")?;
    let launcher_guest_path = normalized_quint_guest_path(&fields, "launcher_guest_path")?;
    let cli_wrapper_guest_path = normalized_quint_guest_path(&fields, "cli_wrapper_guest_path")?;
    let cli_entry_guest_path = normalized_quint_guest_path(&fields, "cli_entry_guest_path")?;
    if files.iter().chain(&shared_libraries).any(|path| {
        !path.is_absolute()
            || path.components().any(|component| {
                matches!(component, std::path::Component::CurDir | std::path::Component::ParentDir)
            })
    }) {
        return Err(NextestDispatchFailure::LaneNotPrepared(
            "Quint runtime manifest contains a non-normalized guest path".to_owned(),
        ));
    }
    let required = [
        &node_guest_path,
        &evaluator_guest_path,
        &launcher_guest_path,
        &cli_wrapper_guest_path,
        &cli_entry_guest_path,
    ];
    if required.iter().any(|path| !files.contains(*path))
        || !files.contains(Path::new("/bin/sh"))
        || !shared_libraries.contains(&node_guest_path)
        || !shared_libraries.contains(&evaluator_guest_path)
        || !shared_libraries.contains(Path::new("/bin/sh"))
        || shared_libraries.len() != 3
        || shared_libraries.iter().any(|path| !files.contains(path))
    {
        return Err(NextestDispatchFailure::LaneNotPrepared(
            "Quint runtime manifest has an incomplete ELF shared-library closure".to_owned(),
        ));
    }
    Ok(PreparedQuintRuntime { files, shared_libraries })
}

fn quint_runtime_guest_files(
    prepared: &Path,
    runtime: &PreparedQuintRuntime,
) -> Result<Vec<GuestFile>, NextestDispatchFailure> {
    let files_root = prepared.join(QUINT_RUNTIME_FILES_DIR);
    let mut guest_files = Vec::with_capacity(runtime.files.len());
    for guest in &runtime.files {
        let relative = guest.strip_prefix("/").map_err(|error| {
            NextestDispatchFailure::LaneNotPrepared(format!(
                "invalid Quint guest path {}: {error}",
                guest.display()
            ))
        })?;
        let host = files_root.join(relative);
        if !host.is_file() {
            return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                "prepared Quint input {} is absent at {}",
                guest.display(),
                host.display()
            )));
        }
        guest_files.push(GuestFile {
            host,
            guest: guest.clone(),
            with_shared_libraries: runtime.shared_libraries.contains(guest),
        });
    }
    Ok(guest_files)
}

/// The `quint` profile's inputs: the prepared Quint runtime and the whole
/// `specs/quint` tree of the selected checkout, each file at the path it has on
/// the host so the test's relative spec path resolves in the guest as it does
/// here.
fn quint_profile_guest_files(current_dir: &Path) -> Result<Vec<GuestFile>, NextestDispatchFailure> {
    let target_dir = cargo_target_dir(&checkout_root());
    let prepared = target_dir.join(QUINT_RUNTIME_DIR);
    let runtime = prepared_quint_runtime(&prepared)?;
    let mut guest_files = quint_runtime_guest_files(&prepared, &runtime)?;
    let tree = current_dir
        .ancestors()
        .map(|ancestor| ancestor.join(QUINT_SPECS_DIR))
        .find(|candidate| candidate.is_dir())
        .ok_or_else(|| {
            NextestDispatchFailure::LaneNotPrepared(format!(
                "no {QUINT_SPECS_DIR} tree above the selected test directory {}",
                current_dir.display()
            ))
        })?;
    stage_tree(&tree, &mut guest_files).map_err(|error| {
        NextestDispatchFailure::LaneNotPrepared(format!(
            "reading the {QUINT_SPECS_DIR} tree {}: {error:?}",
            tree.display()
        ))
    })?;
    Ok(guest_files)
}

fn stage_modules(
    kit: &GuestKit,
    directory: &Path,
    specs: &[(&str, &str)],
    extra: &mut Vec<GuestFile>,
    module_paths: &mut Vec<String>,
) -> Result<(), GuestKitError> {
    for &(relative, guest_path) in specs {
        extra.push(host_module_file(kit, directory, relative, Path::new(guest_path))?);
        module_paths.push(guest_path.to_owned());
    }
    Ok(())
}

fn core_attempt_files(
    kit: &GuestKit,
    directory: &Path,
) -> Result<(Vec<GuestFile>, Vec<String>), GuestKitError> {
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
    stage_modules(kit, directory, &module_specs, &mut extra, &mut module_paths)?;
    Ok((extra, module_paths))
}

fn cgroup_attempt_files(
    kit: &GuestKit,
    directory: &Path,
    environment: AttemptEnvironment,
) -> Result<CgroupAttemptFiles, GuestKitError> {
    let mut extra = cgroup_tools(
        kit,
        directory,
        environment.stages_legb_tools(),
        environment.stages_normal_gate(),
    )?;
    let mut module_paths = Vec::new();
    stage_modules(kit, directory, &cgroup_default_module_specs(), &mut extra, &mut module_paths)?;
    if environment.stages_dummy_module() {
        module_paths.push(DUMMY_MODULE_GUEST_PATH.to_owned());
    }
    Ok(CgroupAttemptFiles { extra, module_paths })
}

fn mtls_attempt_files(
    kit: &GuestKit,
    directory: &Path,
    environment: AttemptEnvironment,
) -> Result<MtlsAttemptFiles, GuestKitError> {
    let tests = InGuestTests::prepared("overdrive-dataplane", "overdrive_dataplane")?;
    let mut extra = dataplane_native_tools();
    extra.push(GuestFile {
        host: tests.binary,
        guest: PathBuf::from(DATAPLANE_LIB_TESTS_PATH),
        with_shared_libraries: true,
    });
    extra.push(workspace_bpf_artifact());
    extra.push(python_stdlib_archive(directory)?);
    extra.extend(tcpdump_account_files(directory)?);
    extra.push(host_module_file(kit, directory, "net/tls/tls", Path::new(TLS_MODULE_GUEST_PATH))?);
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
    let mut native_paths = Vec::new();
    stage_modules(kit, directory, &module_specs, &mut extra, &mut native_paths)?;
    let native = environment != AttemptEnvironment::MtlsRelay;
    Ok(MtlsAttemptFiles {
        extra,
        module_paths: vec![TLS_MODULE_GUEST_PATH.to_owned()],
        native_module_paths: native_paths,
        setup_native: native,
    })
}

fn prepare_nested_attempt(guest: &mut GuestRun) -> Result<(), GuestKitError> {
    guest.command(&["/usr/bin/mkdir", "-p", "/srv/vm"])?;
    guest.command_with_modules(
        &["/lib/overdrive-testing/modules/xfs.ko".to_owned()],
        "/usr/bin/losetup",
        &["/dev/loop0", NESTED_REFLINK_IMAGE],
    )?;
    guest.command(&["/usr/bin/mount", "-t", "xfs", "/dev/loop0", "/srv/vm"])?;
    guest.command(&[
        "/usr/bin/mkdir",
        "-p",
        "/sys/fs/cgroup",
        "/sys/fs/bpf",
        "/sys/kernel/security",
    ])?;
    guest.command(&["/usr/bin/mount", "-t", "cgroup2", "none", "/sys/fs/cgroup"])?;
    guest.command(&["/usr/bin/mount", "-t", "bpf", "bpf", "/sys/fs/bpf"])?;
    guest.command(&["/usr/bin/mount", "-t", "securityfs", "securityfs", "/sys/kernel/security"])?;
    guest.command(&["/usr/bin/cat", "/sys/kernel/security/lsm"])?;
    guest.command(&["/usr/bin/sh", "-c", CGROUP_CONTROLLERS])?;
    guest.command(&["/usr/bin/sh", "-c", NETNS_VAR_RUN_COMPAT])?;
    // XFS was loaded explicitly above for losetup. Preserve it in the shared
    // staged module list used by the batch owner, while avoiding a second
    // load in this per-test path. Load the remaining modules before changing
    // the nested device-node permissions, as `run_nested_boundary` does.
    let modules = std::mem::take(&mut guest.module_paths)
        .into_iter()
        .filter(|module| module.as_str() != XFS_MODULE_GUEST_PATH)
        .collect::<Vec<_>>();
    guest.command_with_modules(&modules, "/usr/bin/true", &[])?;
    guest.command(&["/usr/bin/chmod", "0660", "/dev/kvm", "/dev/vhost-vsock"])?;
    guest.command(&[
        "/usr/bin/stat",
        "-c",
        "NEXTTEST-NESTED-DEVICE %n uid=%u gid=%g mode=%a",
        "/dev/kvm",
        "/dev/vhost-vsock",
    ])?;
    Ok(())
}

fn run_cgroup_attempt_setup(
    guest: &mut GuestRun,
    test_name: &str,
    environment: AttemptEnvironment,
) -> Result<(), GuestKitError> {
    let modules = guest.module_paths.clone();
    guest.command_with_modules(&modules, "/usr/bin/sh", &["-c", "true"])?;
    // The scripted guest's PID 1 loads the staged dummy module at boot for
    // every attempt, so every attempt starts with the automatic dummy0 that a
    // production node does not have. Remove it before any body observes the
    // node's interface set (the node-qualification readbacks require `lo`
    // only), not merely for the bodies that later create their own sink.
    guest.command(&["/usr/bin/sh", "-c", DUMMY_AUTO_DEVICE_CLEANUP])?;
    if test_name == DECLARED_LISTENER_TCP_CAPTURE_TEST {
        guest.command(&[
            "/usr/bin/sh",
            "-c",
            "set -eu; /usr/bin/nft --version; /usr/bin/uname -r",
        ])?;
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
    // Every body that creates a netns through iproute2 and reopens it by its
    // `/var/run/netns` path needs the standard link, not only STEP 13-04.
    guest.command(&["/usr/bin/sh", "-c", NETNS_VAR_RUN_COMPAT])?;
    if environment.stages_normal_gate() {
        guest.command(&["/usr/bin/sh", "-c", CONTROL_PLANE_NORMAL_NATIVE_SETUP])?;
    }
    if environment == AttemptEnvironment::LegbAudit {
        guest.command(&["/usr/bin/sh", "-c", STEP13_05_LEGB_SETUP])?;
        guest.command(&["/usr/bin/sh", "-c", STEP13_05_LEGB_AUDIT])?;
    }
    Ok(())
}

fn nextest_attempt_command(
    module_paths: &[String],
    program: &str,
    test_args: &[String],
    environment: &[(String, String)],
    working_directory: &Path,
    raise_nofile: bool,
) -> Vec<String> {
    let mut command =
        vec![GUEST_PEER_GUEST_PATH.to_owned(), subcommand::RUN_WITH_MODULES.to_owned()];
    command.push(LOOPBACK_MODULE_GUEST_PATH.to_owned());
    command.extend(module_paths.iter().cloned());
    command.push("--".to_owned());
    if raise_nofile {
        command.extend(cgroup_nofile_prefix().map(str::to_owned));
    }
    command.extend([
        "/usr/bin/env".to_owned(),
        "--chdir".to_owned(),
        working_directory.display().to_string(),
    ]);
    command.extend(environment.iter().map(|(key, value)| format!("{key}={value}")));
    command.push(program.to_owned());
    command.extend(test_args.iter().cloned());
    command
}

fn execute_nextest_attempt(
    guest: &mut GuestRun,
    raise_nofile: bool,
    program: &str,
    args: &[String],
    environment: &[(String, String)],
    working_directory: &Path,
    bound: Duration,
) -> Result<i32, GuestKitError> {
    let mut attempt_environment = environment.to_vec();
    attempt_environment.push(("PWD".to_owned(), working_directory.display().to_string()));
    guest.exec_nextest_attempt(
        program,
        args,
        &attempt_environment,
        working_directory,
        raise_nofile,
        bound,
    )
}

/// The nested-KVM `cgroup-default` plan of a per-test attempt: the substrate
/// `nested_vm_test_support` stages, plus the support a body in
/// `NESTED_GUEST_LAUNCH` resolves through the outer guest's own filesystem.
fn nested_cgroup_default_plan(
    kit: &GuestKit,
    support_dir: &Path,
    environment: AttemptEnvironment,
    resources: NestedVmGuestResources,
) -> Result<NextestGuestPlan, GuestKitError> {
    let (mut extra, module_paths) = nested_vm_test_support(kit, support_dir)?;
    if environment.launches_guests() {
        nested_guest_launch_support(support_dir, environment, &mut extra)?;
    }
    Ok(NextestGuestPlan {
        name: "cgroup-default",
        cid: 6,
        extra,
        module_paths,
        memory_mib: resources.memory_mib,
        vcpus: resources.vcpus,
        environment,
        cgroup: false,
        mtls_native_paths: Vec::new(),
        setup_mtls_native: false,
        nested: true,
    })
}

fn nextest_guest_plan(
    kit: &GuestKit,
    support_dir: &Path,
    group: &str,
    environment: AttemptEnvironment,
) -> Result<Option<NextestGuestPlan>, GuestKitError> {
    let standard = |name, cid, extra, module_paths, memory_mib, vcpus| NextestGuestPlan {
        name,
        cid,
        extra,
        module_paths,
        memory_mib,
        vcpus,
        environment,
        cgroup: false,
        mtls_native_paths: Vec::new(),
        setup_mtls_native: false,
        nested: false,
    };
    let plan = match group {
        "probe" => standard("probe", 7, base_tools(), Vec::new(), STANDARD_OUTER_GUEST_MIB, 2),
        "core" => {
            let (extra, module_paths) = core_attempt_files(kit, support_dir)?;
            standard("core", 8, extra, module_paths, STANDARD_OUTER_GUEST_MIB, 2)
        }
        "unload" => standard(
            "unload",
            9,
            vec![workspace_forwarder_module(kit)],
            Vec::new(),
            STANDARD_OUTER_GUEST_MIB,
            2,
        ),
        "ulp" => {
            let tls = host_module_file(
                kit,
                support_dir,
                "net/tls/tls",
                Path::new(TLS_MODULE_GUEST_PATH),
            )?;
            standard(
                "ulp",
                10,
                vec![tls],
                vec![TLS_MODULE_GUEST_PATH.to_owned()],
                STANDARD_OUTER_GUEST_MIB,
                2,
            )
        }
        "stress" => standard("stress", 11, base_tools(), Vec::new(), STANDARD_OUTER_GUEST_MIB, 16),
        "mtls-relay" => {
            let files = mtls_attempt_files(kit, support_dir, environment)?;
            NextestGuestPlan {
                name: "mtls-relay",
                cid: 13,
                extra: files.extra,
                module_paths: files.module_paths,
                memory_mib: STANDARD_OUTER_GUEST_MIB,
                vcpus: 2,
                environment,
                cgroup: false,
                mtls_native_paths: files.native_module_paths,
                setup_mtls_native: files.setup_native,
                nested: false,
            }
        }
        "nested-boundary" => {
            let (extra, module_paths) = nested_vm_test_support(kit, support_dir)?;
            NextestGuestPlan {
                nested: true,
                ..standard("nested-boundary", 12, extra, module_paths, STANDARD_OUTER_GUEST_MIB, 8)
            }
        }
        "cgroup-default" if environment.nested_resources().is_some() => {
            let resources = environment.nested_resources().expect("matched nested environment");
            nested_cgroup_default_plan(kit, support_dir, environment, resources)?
        }
        "cgroup-default" => {
            let files = cgroup_attempt_files(kit, support_dir, environment)?;
            NextestGuestPlan {
                name: "cgroup-default",
                cid: 6,
                extra: files.extra,
                module_paths: files.module_paths,
                memory_mib: STANDARD_OUTER_GUEST_MIB,
                vcpus: 8,
                environment,
                cgroup: true,
                mtls_native_paths: Vec::new(),
                setup_mtls_native: false,
                nested: false,
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(plan))
}

fn guest_attempt_files(
    mut extra: Vec<GuestFile>,
    attempt_files: Vec<GuestFile>,
    source_root: &Path,
    binary_id: &str,
    test_name: &str,
) -> Result<Vec<GuestFile>, NextestDispatchFailure> {
    extra.push(tool("env"));
    extra.push(tool("mkdir"));
    extra.extend(attempt_files);
    for name in [".overdrive-metal-source", ".overdrive-metal-checkout.v1"] {
        let source = source_root.join(name);
        if source.is_file() {
            extra.push(GuestFile {
                host: source.clone(),
                guest: source,
                with_shared_libraries: false,
            });
        }
    }
    let mut deduplicated = Vec::with_capacity(extra.len());
    let mut seen = std::collections::HashMap::<PathBuf, PathBuf>::new();
    for file in extra {
        if !file.host.is_file() {
            return Err(NextestDispatchFailure::LaneNotPrepared(format!(
                "required guest input {} is absent for {binary_id}::{test_name}",
                file.host.display()
            )));
        }
        match seen.get(&file.guest) {
            Some(host) if host == &file.host => {}
            Some(_) => {
                return Err(NextestDispatchFailure::GuestExecution(format!(
                    "two selected guest inputs map different files to {}",
                    file.guest.display()
                )));
            }
            None => {
                seen.insert(file.guest.clone(), file.host.clone());
                deduplicated.push(file);
            }
        }
    }
    Ok(deduplicated)
}

fn prepare_nextest_guest(
    guest: &mut GuestRun,
    plan: &mut NextestGuestPlan,
    test_name: &str,
) -> Result<(), GuestKitError> {
    if plan.nested {
        prepare_nested_attempt(guest)?;
        if plan.environment.launches_guests() {
            // The metal host's staging root exists before any body runs and is
            // traversable by the confined VMM uid, and a node that has served
            // before already holds the delegated workload subtree (the metal
            // wrapper pre-creates it the same way, so a body's before/after
            // cgroup observation compares like with like). The outer guest
            // creates both here, before the body makes its own directory under
            // the staging root and before it takes its baseline observation.
            guest.command(&["/usr/bin/sh", "-c", NESTED_GUEST_LAUNCH_NODE_SETUP])?;
        }
        Ok(())
    } else if plan.cgroup {
        run_cgroup_attempt_setup(guest, test_name, plan.environment)
    } else if plan.setup_mtls_native {
        guest.command_with_modules(&plan.mtls_native_paths, "/usr/bin/true", &[])?;
        setup_dataplane_native_guest(guest)?;
        guest.module_paths.append(&mut plan.mtls_native_paths);
        Ok(())
    } else {
        Ok(())
    }
}

/// Runs one attempt in `guest` and then powers the guest off: the per-test
/// path, where the guest ends with its attempt.
fn execute_nextest_guest_attempt(
    mut guest: GuestRun,
    plan: &NextestGuestPlan,
    attempt: &AttemptInvocation<'_>,
) -> Result<i32, NextestDispatchFailure> {
    let result =
        execute_attempt_phases(&mut guest, plan, attempt, NEXTTEST_ATTEMPT_RESPONSE_BOUND)?;
    guest
        .finish("nextest-attempt")
        .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
    Ok(result)
}

/// What one attempt runs inside an already-booted guest.
struct AttemptInvocation<'a> {
    program: &'a str,
    args: &'a [String],
    environment: &'a [(String, String)],
    working_directory: &'a Path,
}

/// The profile's per-test setup, the test body under `body_bound`, and the
/// profile's per-test cleanup, in a guest that stays up afterwards.
fn execute_attempt_phases(
    guest: &mut GuestRun,
    plan: &NextestGuestPlan,
    attempt: &AttemptInvocation<'_>,
    body_bound: Duration,
) -> Result<i32, NextestDispatchFailure> {
    let AttemptInvocation { program, args, environment, working_directory } = *attempt;
    let route_cleanup = if plan.cgroup && plan.environment == AttemptEnvironment::HostRoute {
        guest
            .command(&["/usr/bin/sh", "-c", STEP13_04_ROUTE_SETUP])
            .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
        Some(STEP13_04_ROUTE_CLEANUP)
    } else if plan.cgroup && plan.environment == AttemptEnvironment::RoutedRoute {
        guest
            .command(&["/usr/bin/sh", "-c", STEP13_06_ROUTE_SETUP])
            .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
        Some(STEP13_06_ROUTE_CLEANUP)
    } else if plan.environment == AttemptEnvironment::MtlsFibHit {
        guest
            .command(&["/usr/bin/sh", "-c", XDP_FIB_HIT_SETUP])
            .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
        Some(XDP_FIB_HIT_CLEANUP)
    } else {
        None
    };
    let launch_environment;
    let environment = if plan.nested && plan.environment.launches_guests() {
        let mut with_path = environment.to_vec();
        let path = if plan.environment == AttemptEnvironment::NestedLaunchRustc {
            let sysroot = rust_sysroot()
                .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
            format!("{}/bin:{NESTED_GUEST_LAUNCH_PATH}", sysroot.display())
        } else {
            NESTED_GUEST_LAUNCH_PATH.to_owned()
        };
        with_path.push(("PATH".to_owned(), path));
        launch_environment = with_path;
        launch_environment.as_slice()
    } else {
        environment
    };
    let attempt_result = execute_nextest_attempt(
        guest,
        plan.name == CGROUP_DEFAULT_GROUP,
        program,
        args,
        environment,
        working_directory,
        body_bound,
    );
    if plan.cgroup && plan.environment == AttemptEnvironment::LegbAudit {
        let audit_result = guest.command(&["/usr/bin/sh", "-c", STEP13_05_LEGB_AUDIT]);
        if attempt_result.as_ref().is_ok_and(|status| *status == 0) {
            audit_result
                .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
        }
    }
    if let Some(cleanup) = route_cleanup {
        let cleanup_result = guest.command(&["/usr/bin/sh", "-c", cleanup]);
        if attempt_result.as_ref().is_ok_and(|status| *status == 0) {
            cleanup_result
                .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
        }
    }
    if plan.cgroup && plan.environment.stages_normal_gate() {
        let cleanup_result =
            guest.command(&["/usr/bin/sh", "-c", CONTROL_PLANE_NORMAL_NATIVE_CLEANUP]);
        if attempt_result.as_ref().is_ok_and(|status| *status == 0) {
            cleanup_result
                .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
        }
    }
    attempt_result.map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))
}

/// What one per-test guest attempt runs: the test binary and its arguments in a
/// guest of `group`, staged for the declared `profile`.
struct GroupAttempt<'a> {
    group: &'a str,
    profile: &'a str,
    program: &'a str,
    args: &'a [String],
    binary_id: &'a str,
}

fn run_nextest_group_attempt(
    attempt: &GroupAttempt<'_>,
    test_name: &str,
) -> Result<i32, NextestDispatchFailure> {
    let GroupAttempt { group, program, args, .. } = *attempt;
    let mut environment = nextest_environment()?;
    environment.push((IN_GUEST_ENV.to_owned(), "1".to_owned()));
    let BootedGroupGuest { guest, plan, current_dir, started, staged, booted } =
        boot_group_guest(attempt, &environment, test_name)?;
    let invocation = AttemptInvocation {
        program,
        args,
        environment: &environment,
        working_directory: &current_dir,
    };
    let outcome = execute_nextest_guest_attempt(guest, &plan, &invocation);
    eprintln!(
        "OVERDRIVE_GUEST_KIT_TIMING group={group} test={test_name} staging_ms={} boot_ms={} total_ms={}",
        staged.as_millis(),
        booted.saturating_sub(staged).as_millis(),
        started.elapsed().as_millis(),
    );
    outcome
}

/// A group guest that is booted, set up for its first test and ready to exec.
struct BootedGroupGuest {
    guest: GuestRun,
    plan: NextestGuestPlan,
    current_dir: PathBuf,
    started: Instant,
    staged: Duration,
    booted: Duration,
}

/// Stages the selected binary and the group's files, boots a guest of the
/// group and runs the group's boot-time setup for `test_name`.
fn boot_group_guest(
    attempt: &GroupAttempt<'_>,
    environment: &[(String, String)],
    test_name: &str,
) -> Result<BootedGroupGuest, NextestDispatchFailure> {
    let GroupAttempt { group, profile, program, binary_id, .. } = *attempt;
    let started = Instant::now();
    let (mut attempt_files, current_dir) = nextest_attempt_files(program, environment)?;
    if profile == QUINT_PROFILE {
        attempt_files.extend(quint_profile_guest_files(&current_dir)?);
    }
    let source_root =
        PathBuf::from(std::env::var("OVERDRIVE_METAL_SOURCE_ROOT").unwrap_or_default());
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).map_err(|error| {
        if matches!(error, GuestKitError::LaneNotPrepared { .. }) {
            NextestDispatchFailure::LaneNotPrepared(format!("{error:?}"))
        } else {
            NextestDispatchFailure::GuestExecution(format!("{error:?}"))
        }
    })?;
    let support = WorkDir::create(&staging, &format!("nextest-{group}-support"))
        .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
    let selected = AttemptEnvironment::of_profile(group, profile)?;
    let Some(mut plan) = nextest_guest_plan(&kit, support.path(), group, selected)
        .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?
    else {
        return Err(NextestDispatchFailure::LaneNotPrepared(format!(
            "guest group {group:?} has no prepared attempt context"
        )));
    };
    plan.extra =
        guest_attempt_files(plan.extra, attempt_files, &source_root, binary_id, test_name)?;
    let staged = started.elapsed();
    let mut guest = GuestRun::boot_with_files(
        &kit,
        &format!("nextest-{}", plan.name),
        plan.cid,
        std::mem::take(&mut plan.extra),
        std::mem::take(&mut plan.module_paths),
        plan.memory_mib,
        plan.vcpus,
    )
    .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
    let booted = started.elapsed();
    prepare_nextest_guest(&mut guest, &mut plan, test_name)
        .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
    // The guest image contains the selected ELF and nextest helpers, not the
    // host checkout tree. Materialize only Nextest's exact selected CWD so the
    // existing `env --chdir` can preserve it without copying host files.
    let guest_working_directory = current_dir.display().to_string();
    guest
        .command(&["/usr/bin/mkdir", "-p", "--", &guest_working_directory])
        .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
    Ok(BootedGroupGuest { guest, plan, current_dir, started, staged, booted })
}

/// The environment a client hands to a shared guest for one test: the
/// selection an isolated attempt runs under, the in-guest marker, and every
/// `OVERDRIVE_` variable of the attaching test (the test's own configuration,
/// including the Metal run variables).
pub(crate) fn shared_request_environment() -> Result<Vec<(String, String)>, String> {
    let mut environment = nextest_environment().map_err(|failure| match failure {
        NextestDispatchFailure::LaneNotPrepared(detail)
        | NextestDispatchFailure::GuestExecution(detail) => detail,
    })?;
    environment.push((IN_GUEST_ENV.to_owned(), "1".to_owned()));
    for (key, value) in std::env::vars_os() {
        let (Ok(key), Ok(value)) = (key.into_string(), value.into_string()) else { continue };
        if key.starts_with("OVERDRIVE_") && !environment.iter().any(|(known, _)| *known == key) {
            environment.push((key, value));
        }
    }
    environment.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(environment)
}

/// What a shared owner needs to boot the guest of one scope.
pub(crate) struct SharedBootRequest<'a> {
    pub(crate) group: &'a str,
    pub(crate) profile: &'a str,
    pub(crate) program: &'a str,
    pub(crate) binary_id: &'a str,
    pub(crate) environment: &'a [(String, String)],
    /// The test the boot is started for; the group's staging may depend on it.
    pub(crate) test_name: &'a str,
}

/// What one test of a shared guest runs.
pub(crate) struct SharedExecRequest<'a> {
    pub(crate) program: &'a str,
    pub(crate) environment: &'a [(String, String)],
    pub(crate) working_directory: &'a Path,
    pub(crate) test_name: &'a str,
}

/// How one exec in a shared guest ended; the console is the guest console from
/// the exec's start.
pub(crate) enum SharedExecOutcome {
    /// The test process exited with this status.
    Exited { status: i32, console: String },
    /// The exec did not return within its bound.
    BoundElapsed { console: String },
    /// The guest went away under the exec.
    Lost { console: String, detail: String },
}

/// A booted guest that serves one test after another: the shared-guest owner's
/// handle on its guest.
pub(crate) struct SharedGuest {
    guest: GuestRun,
    plan: NextestGuestPlan,
}

impl SharedGuest {
    /// Stages and boots a guest of `request.group`, returning it with the
    /// staging and boot durations.
    ///
    /// # Errors
    ///
    /// The detail of the stage that failed.
    pub(crate) fn boot(
        request: &SharedBootRequest<'_>,
    ) -> Result<(Self, Duration, Duration), String> {
        let args = [];
        let attempt = GroupAttempt {
            group: request.group,
            profile: request.profile,
            program: request.program,
            args: &args,
            binary_id: request.binary_id,
        };
        let BootedGroupGuest { guest, plan, staged, booted, .. } =
            boot_group_guest(&attempt, request.environment, request.test_name).map_err(
                |failure| match failure {
                    NextestDispatchFailure::LaneNotPrepared(detail) => {
                        format!("not prepared: {detail}")
                    }
                    NextestDispatchFailure::GuestExecution(detail) => detail,
                },
            )?;
        Ok((Self { guest, plan }, staged, booted.saturating_sub(staged)))
    }

    /// Runs one test in the guest: the profile's setup, the body under
    /// `bound`, the profile's cleanup.
    pub(crate) fn exec(
        &mut self,
        request: &SharedExecRequest<'_>,
        bound: Duration,
    ) -> SharedExecOutcome {
        let started = Instant::now();
        let console_before = self.guest.guest.vm.console().len();
        let console_since = |guest: &GuestRun| {
            let console = guest.guest.vm.console();
            console.get(console_before..).unwrap_or(&console).to_owned()
        };
        // A selected `#[ignore]`d body (`--run-ignored`) must run in the guest too.
        let args = [
            "--exact".to_owned(),
            request.test_name.to_owned(),
            NO_CAPTURE.to_owned(),
            "--include-ignored".to_owned(),
        ];
        let working_directory = request.working_directory.display().to_string();
        let result = self
            .guest
            .command(&["/usr/bin/mkdir", "-p", "--", &working_directory])
            .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))
            .and_then(|()| {
                let invocation = AttemptInvocation {
                    program: request.program,
                    args: &args,
                    environment: request.environment,
                    working_directory: request.working_directory,
                };
                let remaining = bound.saturating_sub(started.elapsed()).max(Duration::from_secs(1));
                execute_attempt_phases(&mut self.guest, &self.plan, &invocation, remaining)
            });
        let console = console_since(&self.guest);
        match result {
            Ok(status) => SharedExecOutcome::Exited { status, console },
            Err(_) if started.elapsed() >= bound => SharedExecOutcome::BoundElapsed { console },
            Err(failure) => {
                let detail = match failure {
                    NextestDispatchFailure::LaneNotPrepared(detail)
                    | NextestDispatchFailure::GuestExecution(detail) => detail,
                };
                if self.has_exited(Duration::from_secs(2)) {
                    SharedExecOutcome::Lost { console, detail }
                } else {
                    // A setup or cleanup command that fails in a live guest is
                    // a failed test, like any nonzero exit.
                    SharedExecOutcome::Exited { status: 1, console: format!("{console}\n{detail}") }
                }
            }
        }
    }

    /// True when the guest's VMM has exited, waiting up to `bound` for it to.
    pub(crate) fn has_exited(&self, bound: Duration) -> bool {
        self.guest.guest.vm.wait_exit(bound).ok().flatten().is_some()
    }
}

/// Runs the lane guest of `fixture` for `test_name` of the test binary `program`
/// (the nextest ELF, re-run in the guest with `args`) and returns the body's
/// exit status; for an owner-side lane the fixture's host script also runs.
///
/// The fixture is looked up first, so an unknown fixture is refused by name
/// before anything is staged or started.
fn run_guest_lane(
    lane_kind: &str,
    fixture: &str,
    program: &str,
    args: &[String],
    binary_id: &str,
    test_name: &str,
) -> Result<i32, NextestDispatchFailure> {
    let Some(placement) =
        INIT_GUEST_LANE_PLACEMENTS.iter().find(|placement| placement.fixture == fixture)
    else {
        return Err(NextestDispatchFailure::LaneNotPrepared(format!(
            "no guest-lane fixture {fixture:?} is known to the guest kit"
        )));
    };
    if placement.kind != lane_kind || placement.test != test_name {
        return Err(NextestDispatchFailure::LaneNotPrepared(format!(
            "the guest-lane fixture {fixture:?} is {kind:?} for {expected:?}, not {lane_kind:?} for {test_name:?}",
            kind = placement.kind,
            expected = placement.test,
        )));
    }
    if binary_id != "overdrive-init::bin/overdrive-init" {
        return Err(NextestDispatchFailure::LaneNotPrepared(format!(
            "guest-lane placement is bound to overdrive-init::bin/overdrive-init, got {binary_id:?}"
        )));
    }
    let environment = nextest_environment()?;
    let (attempt_files, _) = nextest_attempt_files(program, &environment)?;
    let binary = attempt_files.first().map(|file| file.host.clone()).ok_or_else(|| {
        NextestDispatchFailure::LaneNotPrepared(
            "the selected guest-lane ELF is absent from the attempt inputs".to_owned(),
        )
    })?;
    let staging = default_staging_root();
    let kit = GuestKit::provision(&staging).map_err(|error| lane_failure(&error))?;
    let work = WorkDir::create(&staging, &format!("nextest-init-lane-{}", placement.slot))
        .map_err(|error| NextestDispatchFailure::GuestExecution(format!("{error:?}")))?;
    let tests = InGuestTests { binary };
    let attempt =
        lane_attempt(&tests, program, args.to_vec()).map_err(|error| lane_failure(&error))?;
    let cid = INIT_IN_GUEST_LANE.raw(placement.slot);
    let (status, console, host_result) = if placement.kind == "guest-lane-operator" {
        run_in_guest_cell_attempt(&kit, &tests, test_name, cid, work.path(), Some(&attempt))
            .map(|run| (run.exit_status, run.console, None))
    } else {
        run_in_guest_owner_cell_attempt(
            &kit,
            &tests,
            test_name,
            cid,
            work.path(),
            512,
            Some(&attempt),
            |host| run_guest_lane_owner_script(fixture, host),
        )
        .map(|(run, host_result)| (run.exit_status, run.console, Some(host_result)))
    }
    .map_err(|error| lane_failure(&error))?;
    if status != 0 {
        eprintln!("{console}");
        return Ok(status);
    }
    if !console.contains("test result: ok. 1 passed") {
        return Err(NextestDispatchFailure::GuestExecution(format!(
            "guest-lane cell {test_name:?} exited zero without its single-pass libtest summary; console:\n{console}"
        )));
    }
    if let Some(Err(error)) = host_result {
        return Err(NextestDispatchFailure::GuestExecution(format!(
            "existing host script for {fixture:?} failed: {error}; guest console:\n{console}"
        )));
    }
    Ok(0)
}

/// How a guest-kit failure of a lane run is reported: an unprepared artifact
/// is a refusal, anything else a failed run.
fn lane_failure(error: &GuestKitError) -> NextestDispatchFailure {
    if matches!(error, GuestKitError::LaneNotPrepared { .. }) {
        NextestDispatchFailure::LaneNotPrepared(format!("{error:?}"))
    } else {
        NextestDispatchFailure::GuestExecution(format!("{error:?}"))
    }
}

/// The lane run of a test that declares a lane placement on itself: the
/// host-side half of `placement::run_lane`.
pub(crate) fn run_lane_guest(
    placement: &LanePlacement,
    exe: &Path,
    test_name: &str,
) -> Result<(), RunInGuestError> {
    let lane_kind = if placement.is_owner() { "guest-lane-owner" } else { "guest-lane-operator" };
    let program = exe.to_string_lossy().into_owned();
    // A selected `#[ignore]`d body (`--run-ignored`) must run in the guest too.
    let args = [
        "--exact".to_owned(),
        test_name.to_owned(),
        NO_CAPTURE.to_owned(),
        "--include-ignored".to_owned(),
    ];
    let binary_id = std::env::var("NEXTEST_BINARY_ID").unwrap_or_default();
    match run_guest_lane(lane_kind, placement.fixture(), &program, &args, &binary_id, test_name) {
        Ok(0) => Ok(()),
        Ok(status) => Err(RunInGuestError::BodyFailed {
            group: lane_kind.to_owned(),
            test: test_name.to_owned(),
            status,
        }),
        Err(NextestDispatchFailure::LaneNotPrepared(detail)) => {
            Err(RunInGuestError::LaneNotPrepared {
                group: lane_kind.to_owned(),
                test: test_name.to_owned(),
                detail,
            })
        }
        Err(NextestDispatchFailure::GuestExecution(detail)) => {
            Err(RunInGuestError::GuestExecution {
                group: lane_kind.to_owned(),
                test: test_name.to_owned(),
                detail,
            })
        }
    }
}
