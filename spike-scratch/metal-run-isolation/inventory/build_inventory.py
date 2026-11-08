"""Emit DESIGN's closed, source-reviewed placement inventory; never an execution-time classifier."""
import collections,hashlib,json,re
from pathlib import Path
repo=Path.cwd()
meta=json.loads((repo/'docs/feature/metal-run-isolation/spike/f-nextest-list-workspace.stdout').read_text())
targets=json.loads((repo/'docs/feature/metal-run-isolation/spike/local-cargo-targets.json').read_text())
packages={p['name']:Path(p['manifest_path']).parent for p in targets['packages']}
selectors=json.loads((repo/'docs/feature/metal-run-isolation/design/existing-guest-selector-catalogue.json').read_text())['arrays']
fnindex={}
for package,root in packages.items():
 table=collections.defaultdict(list)
 for top in ['src','tests','bin']:
  for file in (root/top).rglob('*.rs'):
   for match in re.finditer(r'(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*\(',file.read_text(errors='replace')):
    table[match[1]].append((file,len(file.read_text(errors='replace')[:match.start()].splitlines())+1))
 fnindex[package]=table

# These are authored placement decisions after reading actual owners and fixtures.
# The tests' names are taken from native nextest metadata, never guessed from paths.
CP_FILE_ONLY={'ca_equivalence','openapi_gate','reconcile_output_validator','reconciler_runtime_view_store','redb_view_store','redb_view_store_no_leak','workflow_journal','workflow_e2e'}
CLI_FILE_ONLY={'cluster_init_removed','deploy_schedule','deploy_stream_request_timeout','endpoint_from_config','post_http_invalid_workload_id','service_submit_streaming_cli_dispatch','submit_jq_pipeline'}
REVIEWED_BINARIES={'overdrive-core::public_api_shape', 'xtask::bin/xtask', 'overdrive-core::service_vip_properties', 'overdrive-fwd::integration', 'overdrive-init::bin/overdrive-init', 'overdrive-core::compile_pass', 'overdrive-fwd-frame-parser-test::differential', 'overdrive-core::newtype_proptest', 'overdrive-control-plane::acceptance', 'overdrive-reconcilers', 'overdrive-netlink', 'overdrive-sim::sim_dataplane_forward_frontend_scoped', 'overdrive-cli', 'overdrive-sim::vm_finalize_failed_ownership_spike', 'overdrive-init::integration', 'overdrive-store-local::acceptance', 'overdrive-cli::acceptance', 'overdrive-core::acceptance', 'overdrive-worker::integration', 'overdrive-testing::bin/overdrive-fwd-guest-suite', 'overdrive-sim::invariant_roundtrip', 'overdrive-control-plane::compile_fail', 'overdrive-core::backend_key', 'overdrive-core::maglev_permutation', 'overdrive-sim::vm_lifecycle_latency_283_spike', 'overdrive-dataplane::bin/verifier_regress', 'overdrive-reconcilers::acceptance', 'overdrive-dataplane::integration', 'overdrive-cli::integration', 'overdrive-cli::bin/coinflip_helper', 'overdrive-control-plane::bin/openapi', 'overdrive-system-conformance', 'overdrive-testing', 'overdrive-core::drop_class', 'overdrive-sim::e09_v2_failure_stream_overlap_spike', 'xtask::dst_lint_async_fs_self_test', 'xtask::dst_lint_live_literal', 'overdrive-core::backend_id', 'overdrive-sim', 'overdrive-gateway::integration', 'xtask::dst_lint_self_test', 'overdrive-testing::bin/overdrive-guest-peer', 'overdrive-core::compile_fail', 'overdrive-dataplane::schema_evolution', 'overdrive-sim::acceptance', 'overdrive-dataplane::acceptance', 'overdrive-sim::bin/dst', 'overdrive-core::service_frontend', 'xtask::integration', 'overdrive-worker::acceptance', 'overdrive-store-local::local_store_edges', 'overdrive-sim::sim_dataplane_iteration', 'overdrive-sim::sim_dataplane_reverse_nat_cross_service', 'overdrive-core::service_frontend_provenance', 'overdrive-sim::sim_dataplane_reverse_nat_per_proto', 'overdrive-core::capture_advertise_port_set_equality', 'overdrive-worker::leg_f_repair', 'overdrive-core::mesh_backend_lb_gate', 'overdrive-sim::harness_determinism', 'overdrive-dataplane::bin/xdp_perf', 'overdrive-store-local::integration', 'overdrive-sim::driver_neutral_allocation_replacement', 'overdrive-core::service_vip', 'overdrive-system-conformance::shared_guest_network_fail_stop_recovery', 'overdrive-worker', 'overdrive-fwd', 'xtask', 'overdrive-control-plane', 'overdrive-control-plane::net_slots_registry', 'overdrive-sim::sim_dataplane_drop_counter', 'overdrive-gateway', 'overdrive-dataplane::aggregate_for_class', 'overdrive-control-plane::integration', 'overdrive-system-conformance::public_ingress_gateway_end_to_end', 'xtask::acceptance', 'overdrive-control-plane::compile_pass', 'overdrive-cli::bin/overdrive', 'overdrive-dataplane::allocator_properties', 'overdrive-core::schema_evolution', 'overdrive-core', 'overdrive-host', 'overdrive-fwd-frame-parser-test', 'overdrive-sim::allocation_restart_write_spike', 'overdrive-host::integration', 'overdrive-sim::e10_vm_early_exit_spike', 'overdrive-sim::integration', 'overdrive-store-local::compile_fail', 'overdrive-store-local', 'overdrive-core::maglev_table_size', 'overdrive-sim::invariant_evaluators', 'overdrive-bpf::integration', 'xtask::dst_lint_infra_subprocess_self_test', 'overdrive-core::service_id', 'overdrive-dataplane'}
NATIVE_HOST={'vmm_equivalence','vsock_beacon_accept_path','vsock_vmm_device'}
HOST_FILE_ONLY={'cgroup_accounting_equivalence','rcgen_ca_chain_verify','rcgen_ca_root_key_envelope'}
operator_cells={
 'transport::real_kernel::the_guest_transport_is_configured_on_the_real_guest_kernel':'the_guest_transport_is_configured_before_ready_in_a_real_guest',
 'transport::real_kernel::the_reserved_intake_ports_refuse_application_binds_of_their_protocol_only':'the_reserved_intake_ports_refuse_application_binds_in_a_real_guest',
 'transport::real_kernel::the_guest_kits_module_release_matches_the_running_kernel':'the_guest_kits_module_matches_the_running_guest_kernel',
 'guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection':'the_guest_accept_path_matches_the_sim_in_a_real_guest',
}
owner_cells={
 'the_v24_kernel_shape_holds_in_both_accept_orders':'the_v24_kernel_shape_holds_in_a_real_guest',
 'a_flow_aborted_before_its_v_g_is_accepted_ends_with_no_application_connection':'a_host_abort_before_the_guest_accepts_ends_the_flow_in_a_real_guest',
 'a_flow_aborted_by_session_loss_ends_and_the_session_is_held_again':'a_session_loss_abort_ends_the_flow_and_the_owner_reconnects_in_a_real_guest',
 'a_flow_the_host_closes_after_the_guest_accepted_it_ends_on_the_guest':'a_host_close_after_the_guest_accepted_ends_the_flow_in_a_real_guest',
 'an_abort_for_a_flow_the_guest_does_not_hold_changes_nothing_and_the_flow_ends_on_the_close':'an_abort_for_an_unheld_flow_is_discarded_in_a_real_guest',
 'a_burst_of_host_aborts_resets_every_application_connection':'a_burst_of_host_aborts_resets_every_connection_in_a_real_guest',
 'tcp_correlation_preserves_exact_identity_and_cleans_transients_for_every_handshake_path':'tcp_correlation_preserves_identity_and_cleanup_in_a_real_guest',
}

def authored(binary,name):
 assert binary in REVIEWED_BINARIES,('unreviewed new target',binary)
 package=binary.split('::')[0];parts=name.split('::');module=parts[1] if parts[0] in ['integration','acceptance'] and len(parts)>1 else parts[0]
 if binary=='overdrive-fwd::integration':
  if 'guest_boundary::' in name: return 'guest-group','nested-boundary','existing named forwarder VM-boundary group'
  if 'unload_in_guest::a_ulp_socket_' in name: return 'guest-group','ulp','existing ULP context without production init holder'
  if 'unload_in_guest::the_unload_stress_' in name: return 'guest-group','stress','existing unload stress group'
  if 'integration::unload::' in name: return 'guest-group','unload','existing unload group'
  if name.startswith('integration::probe_gold::'): return 'guest-group','probe','existing probe isolation setup, one exact selected body'
  return 'guest-group','core','existing core group; exact native-discovered forwarder identity'
 if binary=='overdrive-bpf::integration': return 'guest-group','mtls-relay','real BPF program/map adapter; existing group carries BPF object and kernel dependencies'
 if binary=='overdrive-dataplane::integration':
  if module=='build_rs_artifact_check': return 'host-pure',None,'source-local artifact removal/rebuild; no kernel program load; checkout lock owns it'
  if module=='helpers': return 'host-pure',None,'fixture byte-pattern and transcript helpers'
  return 'guest-group','mtls-relay','real dataplane/kTLS/XDP or adapter-equivalence cell; preserve existing relay/kernel group setup'
 if binary=='overdrive-dataplane':
  selected=any(name.startswith(row['selector']) for row in selectors['DATAPLANE_LIB_NATIVE'])
  if selected: return 'guest-group','mtls-relay','one of the three existing explicitly declared real-BPF library cells'
  return 'host-pure',None,'in-crate policy/decoder/plan tests; the real-BPF library set is declared separately'
 if binary=='overdrive-sim::acceptance' and (name.startswith('acceptance::gateway_relay_exchange::') or name.startswith('acceptance::gateway_application_lifecycle::')):
  return 'guest-group','cgroup-default','actual production Gateway fixture retains a deterministic frontend TCP listener; existing guest context isolates the fixed endpoint'
 if binary=='overdrive-control-plane::acceptance':
  if module in {'job_stop_idempotent','job_stop_intent_key','job_stop_unknown'}: return 'guest-group','cgroup-default','actual common/node_adapters::boot keeps RealCgroupFs; existing CGROUP_ACCEPTANCE'
  return 'host-pure',None,'in-process acceptance over Sim/in-memory ports or private redb/files; no host-bundle boot'
 if binary=='overdrive-control-plane::integration':
  if module in CP_FILE_ONLY: return 'host-pure',None,'read/crypto/filesystem or supplied-port runtime composition; no real node kernel owner'
  return 'guest-group','cgroup-default','real or mixed node boot/network/cgroup module; use existing full node guest support'
 if binary=='overdrive-control-plane':
  if name.startswith('guest_network::flow_io::accept_path_tier3::'): return 'native-launcher',None,'existing host accept seam against a first-level scripted guest, native physical launcher'
  return 'host-pure',None,'source-local pure/Sim compositions; liveness/guest-listener fixtures explicitly use SimVmm/SimBeaconIo/SimCgroupFs'
 if binary=='overdrive-cli::integration':
  if module in CLI_FILE_ONLY: return 'host-pure',None,'parse/render/file or independent HTTP-client fixture; no production node kernel boot'
  return 'guest-group','cgroup-default','real/mixed CLI node or VM composition; existing nested node guest support'
 if binary=='overdrive-worker::integration' or binary=='overdrive-worker::leg_f_repair': return 'guest-group','cgroup-default','real cgroup/netfilter/vsock/namespace worker adapter or mixed module'
 if binary=='overdrive-init::integration': return 'native-launcher',None,'existing host orchestration already boots one fresh guest per cell and owns its script'
 if binary=='overdrive-init::bin/overdrive-init':
  if name in operator_cells: return 'guest-lane-operator',operator_cells[name],'existing production-init operator lane with this exact cell'
  if '::real_kernel::' in name:
   assert name.split('::')[-1] in owner_cells,name
   return 'guest-lane-owner',owner_cells[name.split('::')[-1]],'existing scripted owner lane; retain its exact host script and cell'
  return 'host-pure',None,'source-local Sim/codec/error-mapping tests; no guest-kernel body executed on host'
 if binary=='overdrive-host::integration':
  if module in NATIVE_HOST: return 'native-launcher',None,'direct production VMM or host accept seam; only declared launcher resources'
  if module in HOST_FILE_ONLY: return 'host-pure',None,'synthetic cgroup-shaped files or X.509/AEAD/file fixtures, not real cgroup state'
  return 'guest-group','cgroup-default','real keyring/cgroup host-state or real non-reflink negative fixture; actual kernel/filesystem subject in guest'
 if binary=='overdrive-host':
  if name.startswith('vmm::launch_seccomp_kernel::'): return 'native-launcher',None,'production launch-hook process confinement; bounded per-process launcher state'
  return 'host-pure',None,'pure argument/filter/verdict/error projection and crypto fixtures'
 if binary=='overdrive-gateway::integration': return 'guest-group','cgroup-default','Sim exchange with deliberately fixed loopback/443 resources; guest keeps identical worktrees disjoint'
 if binary=='overdrive-gateway' and name.startswith('application::'): return 'guest-group','cgroup-default','application fixtures include fixed loopback/443 compositions; no cross-run fixed-port alias'
 if package=='overdrive-system-conformance' and 'test' in binary: return 'guest-group','cgroup-default','real node/kernel composition conformance'
 if binary in ['overdrive-system-conformance::public_ingress_gateway_end_to_end','overdrive-system-conformance::shared_guest_network_fail_stop_recovery']: return 'guest-group','cgroup-default','real node/kernel composition conformance'
 return 'host-pure',None,'declared pure/Sim/parser/crypto/private-filesystem/tooling target; no physical test-kernel owner'

rows=[]
for binary,suite in meta['rust-suites'].items():
 package=suite['package-name']
 for name,test in suite['testcases'].items():
  placement,group,rationale=authored(binary,name)
  candidates=[]
  for leaf in reversed(name.split('::')):
   if leaf in fnindex[package]:
    candidates=fnindex[package][leaf];break
  if candidates:
   tokens=set(name.split('::'))
   candidates=sorted(candidates,key=lambda item:(-len(tokens.intersection(item[0].parts)),len(item[0].parts),str(item[0])))
   file,line=candidates[0]
  else:
   # Macro-generated test functions retain their invocation identifier in source.
   leaf=name.split('::')[-1]; found=[]
   for top in ['src','tests','bin']:
    for candidate in (packages[package]/top).rglob('*.rs'):
     body=candidate.read_text(errors='replace');match=re.search(r'\b'+re.escape(leaf)+r'\b',body)
     if match:found.append((candidate,len(body[:match.start()].splitlines())+1))
   if found:
    tokens=set(name.split('::'));found.sort(key=lambda item:(-len(tokens.intersection(item[0].parts)),str(item[0])))
    file,line=found[0]
   else:
    file=packages[package]/('src/lib.rs' if suite['kind']=='lib' else 'Cargo.toml');line=1
  rows.append({'binary_id':binary,'test_name':name,'ignored':test['ignored'],'placement':placement,'context':group,'source':str(file.relative_to(repo))+':'+str(line),'rationale':rationale})
assert len(rows)==meta['test-count']==4640
assert len({(r['binary_id'],r['test_name']) for r in rows})==len(rows)
assert all(r['placement'] in {'host-pure','native-launcher','guest-group','guest-lane-operator','guest-lane-owner'} for r in rows)
counts=collections.Counter(r['placement'] for r in rows)
record={'format':'metal-test-placement-inventory-v1','status':'proposed, not approved','native_run_id':'8d05ef07c5e1f3218a36aeec','discovery_command':'cargo nextest list --workspace --features integration-tests,kvm-tests --message-format json','native_nextest_version':'0.9.149','discovered_binary_count':len(meta['rust-suites']),'test_count':len(rows),'unclassified_count':0,'counts':dict(counts),'rows':rows}
Path('docs/feature/metal-run-isolation/design/test-placement-inventory.json').write_text(json.dumps(record,separators=(',',':'))+'\n')
print(json.dumps({'tests':len(rows),'binaries':len(meta['rust-suites']),'counts':dict(counts),'unclassified':0},sort_keys=True))
