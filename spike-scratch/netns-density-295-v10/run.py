#!/usr/bin/env python3
# V-10 launcher (THROWAWAY SPIKE). Honors the metal hazard rule: it does NOT use
# `cargo xtask metal run` (whose sync rsyncs the whole shared tree with
# --delete). It tars the probe sources to a fresh /tmp/v10-<ts>/ on the host,
# runs driver.sh under the CANONICAL flock lease
# (/run/lock/overdrive-metal-shared.lock) so no other metal writer collides,
# then tars evidence back. Target address is redacted from all streamed output.
import io, os, pathlib, shlex, subprocess, sys, tarfile, time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
env = {}
for line in (ROOT / ".env").read_text().splitlines():
    if line.startswith("OVERDRIVE_METAL_") and "=" in line:
        k, v = line.split("=", 1)
        env.setdefault(k, v.strip().strip('"').strip("'"))
TARGET = env["OVERDRIVE_METAL_TARGET"]
ADDR = TARGET.split("@")[-1]
GK = env.get("OVERDRIVE_METAL_KERNEL", "/srv/vm/overdrive-testing/kernel")
LOCK = "/run/lock/overdrive-metal-shared.lock"
TS = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
REMOTE = f"/tmp/v10-{TS}"
SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", TARGET]


def redact(s):
    return s.replace(ADDR, "<metal>")


SRC = ["forge/Makefile", "forge/vsock_forge.c", "host_listener.c", "guest_client.c",
       "capture.py", "guest-init.sh", "driver.sh", "README.md"]

print(f"[run] remote={REMOTE} target=<metal> lock={LOCK}", flush=True)

# --- ship sources (tar over ssh; no rsync, no touch of ~/overdrive) ---
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode="w:gz") as tar:
    for rel in SRC:
        p = HERE / rel
        if p.exists():
            tar.add(p, arcname=rel)
buf.seek(0)
subprocess.run(SSH + [f"mkdir -p {REMOTE}"], check=True)
p = subprocess.run(SSH + [f"tar -C {REMOTE} -xzf -"], input=buf.read(), check=True)
print("[run] sources shipped", flush=True)

# --- run driver under the canonical flock lease, as root, streaming ---
inner = (f"touch {LOCK} && OVERDRIVE_METAL_KERNEL={GK} "
         f"flock -w 1800 {LOCK} bash {REMOTE}/driver.sh {REMOTE}")
# ssh joins remote args with spaces and the login shell re-splits, so the whole
# privileged command must arrive as ONE quoted argument.
cmd = SSH + ["sudo sh -c " + shlex.quote(inner)]
print("[run] launching driver under flock lease...", flush=True)
proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
log_path = HERE / f"run-{TS}.log"
with log_path.open("w") as lf:
    for line in proc.stdout:
        line = redact(line)
        sys.stdout.write(line); sys.stdout.flush()
        lf.write(line); lf.flush()
rc = proc.wait()
print(f"[run] driver rc={rc}", flush=True)

# --- retrieve evidence (tar back) ---
# driver.sh runs as root, so the evidence files are root-owned; tar as root.
r = subprocess.run(SSH + [f"sudo tar -C {REMOTE} -czf - evidence"], capture_output=True)
if r.returncode == 0 and r.stdout:
    dst = HERE / "increment-a"
    dst.mkdir(exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(r.stdout), mode="r:gz") as tar:
        for m in tar:
            if not m.isfile():
                continue
            assert ".." not in m.name
            d = dst / m.name
            d.parent.mkdir(parents=True, exist_ok=True)
            data = tar.extractfile(m).read()
            try:
                data = data.decode().replace(ADDR, "<metal>").encode()
            except Exception:
                pass
            d.write_bytes(data)
    print(f"[run] evidence -> {dst}", flush=True)
    rj = dst / "evidence" / "results.json"
    if rj.exists():
        print("==== results.json ====", flush=True)
        print(rj.read_text(), flush=True)
else:
    print(f"[run] evidence retrieval failed rc={r.returncode}: {redact(r.stderr.decode()[:300])}", flush=True)

print(f"[run] NOTE host scratch left at {REMOTE} (in /tmp, disposable). "
      f"Remove with: ssh <metal> 'sudo rm -rf {REMOTE}'", flush=True)
sys.exit(rc)
