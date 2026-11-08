import json, os, subprocess, sys, time, uuid
from pathlib import Path

repo=Path.cwd()
base=repo/'spike-scratch/metal-run-isolation/increment-e'
out=repo/'docs/feature/metal-run-isolation/spike'
out.mkdir(parents=True,exist_ok=True)
os.environ['OVERDRIVE_METAL_DESIGN_REPO']=str(repo)
os.environ['CARGO_TARGET_DIR']=str(repo/'target')
(base/'nextest.toml').write_text('experimental = ["wrapper-scripts"]\n[scripts.wrapper.metal-design]\ncommand = '+json.dumps([str(repo/'target/debug/metal-isolation-wrapper-spike')])+'\n[[profile.default.scripts]]\nfilter = "all()"\nrun-wrapper = "metal-design"\n[profile.default]\nslow-timeout = { period = "60s", terminate-after = 8 }\n')

def run(name,argv,*,expected=(0,),timeout=2400,env=None):
    started=time.time()
    print('DESIGN-COMMAND',name,json.dumps(argv),flush=True)
    with (out/('f-'+name+'.stdout')).open('w') as stdout,(out/('f-'+name+'.stderr')).open('w') as stderr:
        result=subprocess.run(argv,stdout=stdout,stderr=stderr,timeout=timeout,env=env)
    print('DESIGN-RESULT',name,'status='+str(result.returncode),'seconds='+str(round(time.time()-started,3)),flush=True)
    if result.returncode not in expected:
        print((out/('f-'+name+'.stderr')).read_text()[-16000:],flush=True)
        raise SystemExit(result.returncode)
    return result.returncode

run('local-vsock-namespace-probe',['python3','spike-scratch/metal-run-isolation/increment-f/netns_probe.py'],timeout=60)
run('bpf-build',['cargo','xtask','bpf-build'],env=dict(os.environ,OVERDRIVE_BPF_NATIVE='1'))
run('guest-kit-prepare',['cargo','xtask','guest-kit-prepare'])
run('nextest-list-workspace',['cargo','nextest','list','--workspace','--features','integration-tests,kvm-tests','--message-format','json'])
run('wrapper-spike-build',['cargo','build','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target')])
run('wrapper-nextest-pass',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'--config-file',str(base/'nextest.toml'),'-E','test(selected_guest_has_an_independent_kernel_boot_id)','--no-capture'])
run('wrapper-nextest-255',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'--config-file',str(base/'nextest.toml'),'-E','test(selected_guest_status_is_255)','--no-capture'],expected=(100,))
run('wrapper-existing-kernel-cell',['cargo','nextest','run','-p','overdrive-fwd','--test','integration','--features','integration-tests,kvm-tests','--config-file',str(base/'nextest.toml'),'-E','test(integration::probe_gold::the_probe_passes_and_leaves_no_residue_on_a_correctly_provisioned_node)','--no-capture'])
print('DESIGN-WRAPPER-PROOF complete=true',flush=True)

run('local-vsock-wrapper-driver',['python3','spike-scratch/metal-run-isolation/increment-g/driver.py'])
