import json, os, re, shutil, subprocess, sys, tempfile, uuid
from pathlib import Path

mode=sys.argv[1] if len(sys.argv)>1 else '--drive'
if mode=='--post':
    root=Path(sys.argv[2]);receipt=Path(sys.argv[3])
    owned=(root/'owned-runtime-paths').read_text().splitlines() if (root/'owned-runtime-paths').exists() else []
    for item in owned:
        path=Path(item)
        assert path.parent==Path('/run') and re.fullmatch(r'ovd-gk-[0-9]+-[0-9]+',path.name),item
        shutil.rmtree(path,ignore_errors=False)
    shutil.rmtree(root)
    receipt.write_text(json.dumps({'owned_staging_removed':not root.exists(),'owned_runtime_paths_removed':all(not Path(item).exists() for item in owned),'removed_runtime_paths':owned},sort_keys=True))
    sys.exit(0)

repo=Path.cwd();base=repo/'spike-scratch/metal-run-isolation/increment-g';out=repo/'docs/feature/metal-run-isolation/spike'
os.environ['OVERDRIVE_METAL_DESIGN_REPO']=str(repo);os.environ['CARGO_TARGET_DIR']=str(repo/'target')
subprocess.run(['cargo','build','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target')],check=True)
config=base/'timeout-nextest.toml'
config.write_text('experimental = ["wrapper-scripts"]\n[scripts.wrapper.metal-design]\ncommand = '+json.dumps(['python3',str(base/'local_netns_exec.py'),str(repo/'target/debug/metal-isolation-local-vsock-spike')])+'\n[[profile.default.scripts]]\nfilter = "all()"\nrun-wrapper = "metal-design"\n[profile.default]\nslow-timeout = { period = "20s", terminate-after = 1, grace-period = "3s" }\n')
results=[]
for callback in [False,True]:
    label='with-cleanup' if callback else 'without-cleanup'
    root=Path('/srv/vm')/('metal-design-cancel-'+uuid.uuid4().hex)
    receipt=out/('h-'+label+'.json');unit='metal-design-timeout-'+uuid.uuid4().hex[:16]
    command=['systemd-run','--quiet','--pipe','--wait','--collect','--unit='+unit,'--property=Type=exec','--property=KillMode=mixed','--property=TimeoutStopSec=10s','--property=WorkingDirectory='+str(repo),'--setenv=HOME='+os.environ['HOME'],'--setenv=PATH='+os.environ['PATH'],'--setenv=OVERDRIVE_METAL_DESIGN_REPO='+str(repo),'--setenv=CARGO_TARGET_DIR='+str(repo/'target'),'--setenv=METAL_DESIGN_RUN_ROOT='+str(root)]
    if callback:
        command+=['--property=ExecStopPost=/usr/bin/python3 '+str(Path(__file__).resolve())+' --post '+str(root)+' '+str(receipt)]
    command+=[shutil.which('cargo'),'nextest','run','--manifest-path',str(base/'Cargo.toml'),'--target-dir',str(repo/'target'),'--config-file',str(config),'-E','test(selected_guest_timeout_owns_resources)','--no-capture']
    try:
        with (out/('h-'+label+'.stdout')).open('w') as stdout,(out/('h-'+label+'.stderr')).open('w') as stderr:
            result=subprocess.run(command,stdout=stdout,stderr=stderr,timeout=180)
        captured=(out/('h-'+label+'.stdout')).read_text()+(out/('h-'+label+'.stderr')).read_text()
        assert result.returncode==100,(label,result.returncode,captured[-10000:])
        assert 'METAL-DESIGN-VM-UP' in captured and 'TIMEOUT' in captured,(label,'expected ready guest followed by actual nextest timeout',captured[-10000:])
        pids=re.findall(r'METAL-DESIGN-VM-UP .*? pid=([0-9]+)',captured)
        assert pids and all(not Path('/proc',pid).exists() for pid in pids),'VMM descendants must be gone when the actual unit wait returns'
        if callback:
            data=json.loads(receipt.read_text());assert data['owned_staging_removed'] and data['owned_runtime_paths_removed']
        else:
            paths=(root/'owned-runtime-paths').read_text().splitlines()
            data={'owned_staging_survives':root.is_dir(),'owned_runtime_survives':any(Path(item).exists() for item in paths),'runtime_paths':paths}
            assert data['owned_staging_survives'] and data['owned_runtime_survives']
            receipt.write_text(json.dumps(data,sort_keys=True))
        results.append({'case':label,'nextest_status':result.returncode,**data})
    finally:
        subprocess.run(['systemctl','stop',unit],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=15)
        if root.exists():
            paths=(root/'owned-runtime-paths').read_text().splitlines() if (root/'owned-runtime-paths').exists() else []
            for item in paths:
                path=Path(item)
                assert path.parent==Path('/run') and re.fullmatch(r'ovd-gk-[0-9]+-[0-9]+',path.name)
                shutil.rmtree(path,ignore_errors=True)
            shutil.rmtree(root)
print('DESIGN-TIMEOUT-CLEANUP-PROOF',json.dumps({'kernel':os.uname().release,'cases':results},sort_keys=True),flush=True)
