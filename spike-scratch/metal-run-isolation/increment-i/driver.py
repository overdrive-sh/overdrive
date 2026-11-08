import json,os,re,subprocess
from pathlib import Path
repo=Path.cwd();base=repo/'spike-scratch/metal-run-isolation/increment-g';out=repo/'docs/feature/metal-run-isolation/spike'
os.environ['OVERDRIVE_METAL_DESIGN_REPO']=str(repo);os.environ['CARGO_TARGET_DIR']=str(repo/'target')
runner='python3 '+str(Path(__file__).resolve().parent/'target_runner.py')+' '+str(repo/'target/debug/metal-isolation-local-vsock-spike')
os.environ['CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER']=runner
hostns=os.readlink('/proc/self/ns/net')

def run(name,argv,expected=(0,),timeout=1200):
    print('DESIGN-COMMAND',name,json.dumps(argv),flush=True)
    with (out/('i-'+name+'.stdout')).open('w') as stdout,(out/('i-'+name+'.stderr')).open('w') as stderr:
        result=subprocess.run(argv,stdout=stdout,stderr=stderr,timeout=timeout)
    print('DESIGN-RESULT',name,'status='+str(result.returncode),flush=True)
    if result.returncode not in expected:
        print((out/('i-'+name+'.stdout')).read_text()[-10000:]+(out/('i-'+name+'.stderr')).read_text()[-10000:],flush=True)
        raise SystemExit(result.returncode)

run('build',['cargo','build','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target')])
run('pure-sim',['cargo','nextest','run','-p','overdrive-sim','--lib','-E','test(adapters::beacon_io::tests::a_guest_cannot_connect_before_the_listener_is_bound)','--no-capture'])
pure=(out/'i-pure-sim.stdout').read_text()+(out/'i-pure-sim.stderr').read_text()
assert 'METAL-DESIGN-PURE-HOST namespace='+hostns in pure and 'METAL-DESIGN-LOCAL-NAMESPACE' not in pure
run('guest-pass',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'-E','test(selected_guest_has_an_independent_kernel_boot_id)','--no-capture'])
run('guest-255-retry',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'-E','test(selected_guest_status_is_255)','--retries','1','--no-fail-fast','--no-capture'],expected=(100,))
failed=(out/'i-guest-255-retry.stdout').read_text()+(out/'i-guest-255-retry.stderr').read_text()
boots=re.findall(r'METAL-DESIGN-255 boot_id=([0-9a-f-]+)',failed)
assert len(boots)==2 and len(set(boots))==2,('retry must have exactly two fresh guest kernels',boots)
assert len(re.findall(r'child_status=255 ',failed))==2
run('existing-kernel-cell',['cargo','nextest','run','-p','overdrive-fwd','--test','integration','--features','integration-tests,kvm-tests','-E','test(integration::probe_gold::the_probe_passes_and_leaves_no_residue_on_a_correctly_provisioned_node)','--no-capture'])
run('native-direct-fork',['cargo','nextest','run','-p','overdrive-host','--test','integration','--features','integration-tests,kvm-tests','-E','test(integration::vsock_vmm_device::the_fork_boots_and_passes_data_under_the_production_launch_identity)','--no-capture'])
native=(out/'i-native-direct-fork.stdout').read_text()+(out/'i-native-direct-fork.stderr').read_text()
assert 'METAL-DESIGN-NATIVE-DIRECT' in native and '===== disposable guest' not in native
record={'kernel':os.uname().release,'runner_injection':'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER','argv_unchanged':True,'caller_nextest_config_overrides_used':False,'pure_sim_host_namespace':hostns,'retry_guest_boot_ids':boots,'retry_child_statuses':[255,255],'actual_retry_command_status':100,'existing_kernel_cell_in_disposable_guest':True,'native_fork_body_executed_directly_on_physical_kvm':True}
(out/'i-target-runner-result.json').write_text(json.dumps(record,indent=2)+'\n')
print('DESIGN-TARGET-RUNNER-PROOF',json.dumps(record,sort_keys=True),flush=True)
