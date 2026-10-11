#!/usr/bin/env python3
"""V-29 launcher. Runs `python3 <increment>/run.py` on the metal host as root UNDER THE CANONICAL LEASE
(infra/metal/lease-holder.sh over ssh, same protocol as infra/metal/bootstrap.sh) WITHOUT the tree rsync
(`cargo xtask metal run` would rsync --delete over the shared tree). Sources are pushed to /tmp/v29-work by sync.sh.
Evidence under /tmp/v29-work/<increment>/evidence is pulled back afterwards. Usage: launch.py <increment> [args]"""
import base64, io, json, os, pathlib, random, shlex, subprocess, sys, tarfile, time
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
inc = sys.argv[1]; extra = " ".join(sys.argv[2:])
env = {}
for l in (ROOT / ".env").read_text().splitlines():
    if "=" in l and l.startswith("OVERDRIVE_METAL_"):
        k, v = l.split("=", 1); env[k] = v.strip().strip('"').strip("'")
target = env["OVERDRIVE_METAL_TARGET"]; addr = target.split("@")[-1]
red = lambda s: s.replace(target, "<metal-target>").replace(addr, "<metal-address>")
ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", "-o", "StrictHostKeyChecking=accept-new", target]
LOCK, OWNER = "/run/lock/overdrive-metal-shared.lock", "/run/lock/overdrive-metal-shared.owner"
token = f"v29-{os.getpid()}-{int(time.time())}-{random.randrange(10**6)}"
head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
timeout = os.environ.get("V29_LEASE_WAIT", "3600")
helper = base64.b64encode((ROOT / "infra/metal/lease-holder.sh").read_bytes()).decode()
cmd = (f'lease_script=$(printf %s {shlex.quote(helper)} | base64 -d); exec sudo -n bash -c "$lease_script" overdrive-metal-lease '
       + " ".join(shlex.quote(a) for a in (LOCK, OWNER, timeout, token, "run", f"spike-netns-density-295-v29-udp-empty-{inc}", str(HERE), head)))
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
    run = ('sudo -n env HOME=/home/ubuntu PATH=/home/ubuntu/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin '
           f'OVERDRIVE_METAL_KERNEL={env["OVERDRIVE_METAL_KERNEL"]} bash -c ' + shlex.quote(
        f'cd /tmp/v29-work && {{ test -d {inc}/evidence && mv {inc}/evidence {inc}/evidence.prev.$(date +%s); true; }} && '
        f'python3 {inc}/run.py {extra}'))
    local = HERE / "out"; local.mkdir(exist_ok=True)
    with (local / f"launch-{inc}-{int(time.time())}.log").open("w") as out:
        p = subprocess.Popen(ssh + [run], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace")
        for line in p.stdout:
            line = red(line); out.write(line); out.flush(); print(line, end="", flush=True)
        rc = p.wait()
    r = subprocess.run(ssh + [f"sudo -n tar -C /tmp/v29-work/{inc} -czf - evidence 2>/dev/null"], capture_output=True)
    if r.returncode == 0 and r.stdout:
        with tarfile.open(fileobj=io.BytesIO(r.stdout), mode="r:gz") as tar:
            for m in tar:
                if not m.isfile(): continue
                assert ".." not in m.name
                d = HERE / inc / m.name; d.parent.mkdir(parents=True, exist_ok=True)
                data = tar.extractfile(m).read()
                if not m.name.endswith(".pcap"): data = data.replace(addr.encode(), b"<metal-address>")
                d.write_bytes(data)
        print(json.dumps({"retrieved": True, "launcher_rc": rc}))
    else:
        print(json.dumps({"retrieved": False}))
finally:
    try: lease.stdin.close()
    except Exception: pass
    try: lease.wait(timeout=20)
    except Exception: lease.kill()
sys.exit(rc)
