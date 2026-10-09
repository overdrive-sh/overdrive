#!/usr/bin/env python3
"""Launch one V-13 stage on the metal host UNDER THE CANONICAL LEASE (same protocol as
infra/metal/bootstrap.sh: infra/metal/lease-holder.sh over ssh, token + owner metadata), WITHOUT the
tree rsync (which would --delete the shared /home/ubuntu/overdrive that other spikes use).
Files go to /tmp/v13-work/ only. Usage: launch.py <stage> [N]"""
import base64, io, json, os, pathlib, random, shlex, subprocess, sys, tarfile, time
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
stage = sys.argv[1]; extra = sys.argv[2:]
env = {}
for l in (ROOT / ".env").read_text().splitlines():
    if "=" in l and l.startswith("OVERDRIVE_METAL_"):
        k, v = l.split("=", 1); env[k] = v.strip().strip('"').strip("'")
target = env["OVERDRIVE_METAL_TARGET"]; addr = target.split("@")[-1]
red = lambda s: s.replace(target, "<metal-target>").replace(addr, "<metal-address>")
ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", "-o", "StrictHostKeyChecking=accept-new", target]
LOCK, OWNER = "/run/lock/overdrive-metal-shared.lock", "/run/lock/overdrive-metal-shared.owner"
token = f"v13-{os.getpid()}-{int(time.time())}-{random.randrange(10**6)}"
head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
timeout = os.environ.get("V13_LEASE_WAIT", "3000")
# 1. sync ONLY our private dir (no --delete outside it)
subprocess.check_call(["ssh", "-o", "BatchMode=yes", target, "mkdir -p /tmp/v13-work"])
for f in ("run.py", "build.sh", "ktls_holder.py"):
    subprocess.check_call(["scp", "-q", "-o", "BatchMode=yes", str(HERE / f), f"{target}:/tmp/v13-work/{f}"])
# 2. take the lease (hold the ssh session's stdin open)
helper = base64.b64encode((ROOT / "infra/metal/lease-holder.sh").read_bytes()).decode()
cmd = (f'lease_script=$(printf %s {shlex.quote(helper)} | base64 -d); exec sudo -n bash -c "$lease_script" overdrive-metal-lease '
       + " ".join(shlex.quote(a) for a in (LOCK, OWNER, timeout, token, "run", f"spike-netns-density-295-v13-{stage}", str(HERE), head)))
lease = subprocess.Popen(ssh + [cmd], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
t0 = time.time()
while True:
    line = lease.stdout.readline()
    if not line:
        print("lease holder exited:", lease.wait()); sys.exit(75)
    print("[lease]", red(line.rstrip()), flush=True)
    if f"OVERDRIVE_METAL_LEASE_ACQUIRED token={token}" in line: break
print(f"[lease] acquired after {time.time()-t0:.0f}s", flush=True)
rc = 1
try:
    run = (f'sudo -n env HOME=/home/ubuntu PATH=/home/ubuntu/.cargo/bin:$PATH OVERDRIVE_METAL_KERNEL={env["OVERDRIVE_METAL_KERNEL"]} '
           f'python3 /tmp/v13-work/run.py {stage} {" ".join(extra)}')
    local = HERE / "out"; local.mkdir(exist_ok=True)
    with (local / f"launch-{stage}-{int(time.time())}.log").open("w") as out:
        p = subprocess.Popen(ssh + [run], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        for line in p.stdout:
            line = red(line); out.write(line); out.flush(); print(line, end="", flush=True)
        rc = p.wait()
    # 3. retrieve the evidence directories of this stage
    r = subprocess.run(ssh + [f"sudo -n tar -C /tmp/v13-work -czf - evidence"], capture_output=True)
    if r.returncode == 0:
        dst = HERE
        with tarfile.open(fileobj=io.BytesIO(r.stdout), mode="r:gz") as tar:
            for m in tar:
                if not m.isfile(): continue
                assert ".." not in m.name
                d = dst / m.name; d.parent.mkdir(parents=True, exist_ok=True)
                d.write_bytes(tar.extractfile(m).read().replace(addr.encode(), b"<metal-address>"))
        print(json.dumps({"retrieved": True, "launcher_rc": rc}))
    else:
        print(json.dumps({"retrieved": False, "stderr": red(r.stderr.decode())[:300]}))
finally:
    try: lease.stdin.close()
    except Exception: pass
    try: lease.wait(timeout=20)
    except Exception: lease.kill()
sys.exit(rc)
