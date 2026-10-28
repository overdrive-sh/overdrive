//! Runs the existing `overdrive-fwd` integration scenarios, the unchanged
//! `overdrive-dataplane` inbound guest-relay cells, and the already-authored
//! STEP 13-04 real-kernel cells inside disposable `GuestKit` Cloud Hypervisor
//! guests. Test-support executable only; never shipped by a production crate.
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
const STRESS: &str = "integration::unload_in_guest::the_unload_stress_never_faults_in_a_guest";
const NFNETLINK_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nfnetlink.ko";
const NF_TABLES_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/nf_tables.ko";
const VETH_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/veth.ko";
const TLS_MODULE_GUEST_PATH: &str = "/lib/overdrive-testing/modules/tls.ko";
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
const NESTED_REFLINK_IMAGE: &str = "/var/lib/overdrive-testing/reflink.xfs";
const NESTED_PREBUILT_INIT: &str = "/opt/overdrive-testing/prebuilt-overdrive-init";
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

type Cell = (&'static str, usize, bool);
type CgroupManifests<'a> = (&'a [Cell], &'a [Cell], &'a [Cell], &'a [Cell]);

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

const CGROUP_DEFAULT_BASELINE_BODIES: usize = 55;
const STEP13_04_NATIVE_BODIES: usize = 18;
const CGROUP_DEFAULT_TOTAL_BODIES: usize = CGROUP_DEFAULT_BASELINE_BODIES + STEP13_04_NATIVE_BODIES;

struct GuestRun {
    guest: overdrive_testing::guest_kit::ScriptedGuest,
    module_paths: Vec<String>,
    _work: WorkDir,
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
        let console_before = self.guest.vm.console().len();
        let mut command = vec![GUEST_PEER_GUEST_PATH, subcommand::RUN_WITH_MODULES];
        command.push(LOOPBACK_MODULE_GUEST_PATH);
        command.extend(self.module_paths.iter().map(String::as_str));
        command.extend(["--", IN_GUEST_TESTS_PATH]);
        command.extend_from_slice(test_args);
        let code = self.guest.peer().exec(&command, bound)?;
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
        Ok(())
    }

    fn exec_cgroup(
        &mut self,
        test_args: &[&str],
        expected_passed: usize,
        bound: Duration,
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
        let code = self.guest.peer().exec(&command, bound)?;
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

fn cgroup_tools(kit: &GuestKit, stage_step13_04_nft: bool) -> Vec<GuestFile> {
    let mut files: Vec<_> =
        ["cat", "chmod", "ip", "mkdir", "mount", "sh", "stat"].into_iter().map(tool).collect();
    if stage_step13_04_nft {
        files.extend(["ln", "nft", "ss", "uname"].into_iter().map(tool));
    }
    let sleep = tool("sleep");
    files.push(GuestFile { guest: PathBuf::from("/bin/sleep"), ..sleep });
    // The unchanged bodies run the existing FWD-PREREQ identity check at its
    // workspace contract path. Stage the same GuestKit-verified module image
    // there; the image also keeps its normal /lib/overdrive copy and guest
    // boot owns the load. Nothing touches the physical host module.
    files.push(workspace_forwarder_module(kit));
    files
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
    let staging = default_staging_root();
    let work = WorkDir::create(&staging, "fwd-suite-mtls-relay-module")?;
    let tls = host_module_file(kit, work.path(), "net/tls/tls", Path::new(TLS_MODULE_GUEST_PATH))?;
    let mut guest = GuestRun::boot(
        kit,
        &tests,
        "mtls-relay",
        13,
        vec![tls],
        vec![TLS_MODULE_GUEST_PATH.to_owned()],
    )?;
    guest.exec(&[TEST_THREADS, NO_CAPTURE, MTLS_RELAY], 7, CELL_BOUND)?;
    drop(work);
    guest.finish("mtls-relay")
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

fn run_cgroup_binary(
    kit: &GuestKit,
    package: &str,
    target: &str,
    name: &str,
    slot: u32,
    manifests: CgroupManifests<'_>,
    stage_step13_04_nft: bool,
) -> Result<(usize, usize), GuestKitError> {
    let (baseline_cells, step13_04_cells, routed_step13_04_cells, post_route_step13_04_cells) =
        manifests;
    let tests = InGuestTests::prepared(package, target)?;
    let staging = default_staging_root();
    let modules = WorkDir::create(&staging, &format!("fwd-suite-{name}-kernel-modules"))?;
    let mut extra = cgroup_tools(kit, stage_step13_04_nft);
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
    let mut guest = GuestRun::boot(kit, &tests, name, slot, extra, module_paths)?;
    let guest_modules = guest.module_paths.clone();
    guest.command_with_modules(&guest_modules, "/usr/bin/sh", &["-c", "true"])?;
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
    guest.finish(name)?;
    drop(modules);
    Ok((baseline_passed, step13_04_passed))
}

fn run_cgroup_default(kit: &GuestKit, _fwd_tests: &InGuestTests) -> Result<(), GuestKitError> {
    let acceptance = run_cgroup_binary(
        kit,
        "overdrive-control-plane",
        "acceptance",
        "cgroup-control-plane-acceptance",
        14,
        (CGROUP_ACCEPTANCE, &[], &[], &[]),
        false,
    )?;
    let control_plane = run_cgroup_binary(
        kit,
        "overdrive-control-plane",
        "integration",
        "cgroup-control-plane-integration",
        15,
        (CGROUP_CONTROL_PLANE, STEP13_04_CONTROL_PLANE, &[], &[]),
        false,
    )?;
    let worker = run_cgroup_binary(
        kit,
        "overdrive-worker",
        "integration",
        "cgroup-worker-integration",
        6,
        (CGROUP_WORKER, STEP13_04_WORKER, STEP13_04_HOST_LOCAL_FIREWALL, STEP13_04_REMOTE_FIREWALL),
        true,
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
    let total_passed = baseline_passed + step13_04_passed;
    if total_passed != CGROUP_DEFAULT_TOTAL_BODIES {
        return Err(GuestKitError::Staging {
            detail: format!(
                "cgroup-default selected {total_passed} passing bodies in total, expected exactly {CGROUP_DEFAULT_TOTAL_BODIES}"
            ),
            source: std::io::Error::other("the combined cgroup-default manifest changed"),
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
