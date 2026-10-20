#!/usr/bin/env python3
"""ev-ctl spike launcher. Run one command on the metal host as root UNDER THE CANONICAL LEASE (same protocol as infra/metal/bootstrap.sh:
infra/metal/lease-holder.sh over ssh, token + owner metadata), WITHOUT the tree rsync (which would --delete the shared
checkout). Files live in /var/tmp/evctl-work only (pushed by sync.sh). /var/tmp/evctl-work/evidence is pulled back afterwards
(address redacted). Usage: launch.py <name> <remote command...>"""
import base64, io, json, os, pathlib, random, shlex, subprocess, sys, tarfile, time
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
name = sys.argv[1]; remote = " ".join(sys.argv[2:])
env = {}
for l in (ROOT / ".env").read_text().splitlines():
    if "=" in l and l.startswith("OVERDRIVE_METAL_"):
        k, v = l.split("=", 1); env[k] = v.strip().strip('"').strip("'")
target = env["OVERDRIVE_METAL_TARGET"]; addr = target.split("@")[-1]
red = lambda s: s.replace(target, "<metal-target>").replace(addr, "<metal-address>")
ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=40", "-o", "StrictHostKeyChecking=accept-new", target]
LOCK, OWNER = "/run/lock/overdrive-metal-shared.lock", "/run/lock/overdrive-metal-shared.owner"
token = f"evctl-{os.getpid()}-{int(time.time())}-{random.randrange(10**6)}"
head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
timeout = os.environ.get("EVCTL_LEASE_WAIT", "120")
helper = base64.b64encode((ROOT / "infra/metal/lease-holder.sh").read_bytes()).decode()
cmd = (f'lease_script=$(printf %s {shlex.quote(helper)} | base64 -d); exec sudo -n bash -c "$lease_script" overdrive-metal-lease '
       + " ".join(shlex.quote(a) for a in (LOCK, OWNER, timeout, token, "run", f"spike-netns-density-295-ev-ctl-{name}", str(HERE), head)))
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
           f'bash -c {shlex.quote(remote)}')
    local = HERE / "out"; local.mkdir(exist_ok=True)
    with (local / f"launch-{name}-{int(time.time())}.log").open("w") as out:
        p = subprocess.Popen(ssh + [run], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        for line in p.stdout:
            line = red(line); out.write(line); out.flush(); print(line, end="", flush=True)
        rc = p.wait()
    r = subprocess.run(ssh + ["sudo -n tar -C /var/tmp/evctl-work -czf - evidence 2>/dev/null"], capture_output=True)
    if r.returncode == 0 and r.stdout:
        with tarfile.open(fileobj=io.BytesIO(r.stdout), mode="r:gz") as tar:
            for m in tar:
                if not m.isfile(): continue
                assert ".." not in m.name
                d = HERE / m.name; d.parent.mkdir(parents=True, exist_ok=True)
                d.write_bytes(tar.extractfile(m).read().replace(addr.encode(), b"<metal-address>"))
        print(json.dumps({"retrieved": True, "launcher_rc": rc}))
    else:
        print(json.dumps({"retrieved": False}))
finally:
    try: lease.stdin.close()
    except Exception: pass
    try: lease.wait(timeout=20)
    except Exception: lease.kill()
sys.exit(rc)
