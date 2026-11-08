import json, os, re, subprocess, time
from pathlib import Path
repo=Path.cwd();base=repo/'spike-scratch/metal-run-isolation/increment-g';out=repo/'docs/feature/metal-run-isolation/spike'
os.environ['OVERDRIVE_METAL_DESIGN_REPO']=str(repo)
os.environ['CARGO_TARGET_DIR']=str(repo/'target')
config=base/'nextest.toml'
config.write_text('experimental = ["wrapper-scripts"]\n[scripts.wrapper.metal-design]\ncommand = '+json.dumps(['python3',str(base/'local_netns_exec.py'),str(repo/'target/debug/metal-isolation-local-vsock-spike')])+'\n[[profile.default.scripts]]\nfilter = "all()"\nrun-wrapper = "metal-design"\n[profile.default]\nslow-timeout = { period = "60s", terminate-after = 8 }\n')

def run(name,argv,expected=(0,),timeout=2400):
    print('DESIGN-COMMAND',name,json.dumps(argv),flush=True)
    with (out/('g-'+name+'.stdout')).open('w') as stdout,(out/('g-'+name+'.stderr')).open('w') as stderr:
        result=subprocess.run(argv,stdout=stdout,stderr=stderr,timeout=timeout)
    print('DESIGN-RESULT',name,'status='+str(result.returncode),flush=True)
    if result.returncode not in expected:
        print((out/('g-'+name+'.stderr')).read_text()[-16000:],flush=True)
        raise SystemExit(result.returncode)

def snapshot():
    modules={line.split()[0]:line.split()[1] for line in Path('/proc/modules').read_text().splitlines()}
    def command(argv):
        result=subprocess.run(argv,capture_output=True,text=True,check=True)
        return json.loads(result.stdout)
    nft=command(['nft','-j','list','ruleset'])
    nft.get('nftables',[])[:]=[row for row in nft.get('nftables',[]) if 'metainfo' not in row]
    for row in nft.get('nftables',[]):
        for expression in row.get('rule',{}).get('expr',[]):
            if 'counter' in expression:
                expression['counter']={}
    programs=command(['bpftool','-j','prog','show'])
    identities=sorted((row['id'],row['type'],row.get('name'),row.get('tag')) for row in programs)
    cgroup=Path('/sys/fs/cgroup/overdrive.slice/workloads.slice')
    cgroups=sorted(str(p.relative_to(cgroup)) for p in cgroup.rglob('*') if p.is_dir()) if cgroup.is_dir() else None
    return {'modules':modules,'nft':nft,'programs':identities,'routes':command(['ip','-j','route','show','table','all']),'test_cgroups':cgroups,'root_vsock_modes':{name:Path('/proc/sys/net/vsock/'+name).read_text() for name in ['ns_mode','child_ns_mode']}}

run('tool-versions',['bash','-c','uname -r; systemctl --version; cargo nextest --version'])
run('build',['cargo','build','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target')])
before=snapshot()
commands=['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'--config-file',str(config),'-E','test(selected_guest_has_an_independent_kernel_boot_id)','--no-capture']
processes=[]
for index in range(2):
    stdout=(out/f'g-pair-{index}.stdout').open('w');stderr=(out/f'g-pair-{index}.stderr').open('w')
    process=subprocess.Popen(commands,stdout=stdout,stderr=stderr)
    processes.append((process,stdout,stderr))
for process,stdout,stderr in processes:
    status=process.wait(timeout=600)
    stdout.close();stderr.close()
    assert status==0,('paired nextest status',status)
rows=[]
for index in range(2):
    text=(out/f'g-pair-{index}.stdout').read_text()+(out/f'g-pair-{index}.stderr').read_text()
    result=re.search(r'child_status=(\d+) cid=(\d+) cleanup=complete namespace=(net:\[\d+\]) vm_up_ns=(\d+) vm_end_ns=(\d+)',text)
    assert result,('missing exact wrapper result',index,text[-8000:])
    rows.append({'status':int(result[1]),'cid':int(result[2]),'namespace':result[3],'vm_up_ns':int(result[4]),'vm_end_ns':int(result[5]),'guest_boot_id':re.search(r'METAL-DESIGN-GUEST boot_id=([0-9a-f-]+)',text)[1]})
assert rows[0]['namespace']!=rows[1]['namespace'] and rows[0]['cid']==rows[1]['cid']
assert rows[0]['guest_boot_id']!=rows[1]['guest_boot_id']
assert max(row['vm_up_ns'] for row in rows)<min(row['vm_end_ns'] for row in rows),'actual ready guest intervals must overlap'
run('nextest-255',['cargo','nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'--config-file',str(config),'-E','test(selected_guest_status_is_255)','--no-capture'],expected=(100,))
run('existing-kernel-cell',['cargo','nextest','run','-p','overdrive-fwd','--test','integration','--features','integration-tests,kvm-tests','--config-file',str(config),'-E','test(integration::probe_gold::the_probe_passes_and_leaves_no_residue_on_a_correctly_provisioned_node)','--no-capture'])
run('native-direct-fork',['cargo','nextest','run','-p','overdrive-host','--test','integration','--features','integration-tests,kvm-tests','--config-file',str(config),'-E','test(integration::vsock_vmm_device::the_fork_boots_and_passes_data_under_the_production_launch_identity)','--no-capture'])
after=snapshot()
assert before==after,{'physical_host_surfaces_changed':[key for key in before if before[key]!=after[key]]}
result={'kernel':os.uname().release,'actual_guest_intervals_overlap':True,'same_cid_separate_local_namespaces':rows,'physical_host_test_surfaces_unchanged':True,'nextest_status_255_body_aggregate':100}
(out/'g-paired-result.json').write_text(json.dumps(result,indent=2)+'\n')
print('DESIGN-LOCAL-NAMESPACE-WRAPPER-PROOF',json.dumps(result,sort_keys=True),flush=True)
