"""Remove only the root-owned import cache created by attempt a, under canonical lease."""
import pathlib, subprocess, secrets, json, hashlib
token=secrets.token_hex(12)
lease=subprocess.Popen(['bash','/home/ubuntu/overdrive/infra/metal/lease-holder.sh','/run/lock/overdrive-metal-shared.lock','/run/lock/overdrive-metal-shared.owner','1',token,'run','vsock-scale-owned-preflight-cache-cleanup','/Users/marcus/conductor/workspaces/helios/wellington-v2','5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
try:
 line=lease.stdout.readline();assert line.startswith('OVERDRIVE_METAL_LEASE_ACQUIRED'),line
 p=pathlib.Path('/home/ubuntu/overdrive/spike-scratch/netns-density-295-vsock-scale/increment-a/__pycache__/snapshot.cpython-314.pyc')
 receipt={'lease_acquired':line.strip(),'exact_owned_path':str(p),'exists_before':p.exists()}
 if p.exists():receipt['cache_sha256']=hashlib.sha256(p.read_bytes()).hexdigest();p.unlink();p.parent.rmdir()
 receipt['exists_after']=p.exists();print(json.dumps(receipt))
finally:
 lease.stdin.close();lease.wait(timeout=5)
print(json.dumps({'lease_exit':lease.returncode,'owner_exists_after':pathlib.Path('/run/lock/overdrive-metal-shared.owner').exists()}))
