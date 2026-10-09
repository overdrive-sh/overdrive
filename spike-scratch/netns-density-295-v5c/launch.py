#!/usr/bin/env python3
"""Launch one increment on the native metal host WITHOUT `cargo xtask metal run` (its sync rsyncs the whole tree
with --delete and races the other spikes). Copies only this spike directory, runs the increment as root over ssh
under this spike's own flock, retrieves evidence/ and logs (target address redacted).
Usage: python3 spike-scratch/netns-density-295-v5c/launch.py increment-a [args...]"""
import io, json, os, pathlib, subprocess, sys, tarfile, time

SCRATCH = pathlib.Path(__file__).resolve().parent
ROOT = SCRATCH.parents[1]
attempt = sys.argv[1]
assert attempt.startswith("increment-") and "/" not in attempt
inc = SCRATCH / attempt
ev = inc / "evidence"
assert not ev.exists(), "increments are immutable: use a fresh increment"
local = SCRATCH / "out" / f"{attempt}-launch"
local.mkdir(parents=True, exist_ok=True)

env = os.environ.copy()
for entry in (ROOT / ".env").read_text().splitlines():
    if "=" in entry and entry.startswith("OVERDRIVE_METAL_"):
        k, v = entry.split("=", 1)
        env.setdefault(k, v.strip().strip('"').strip("'"))
target = env["OVERDRIVE_METAL_TARGET"]
address = target.split("@")[-1]
REMOTE = "/home/ubuntu/overdrive/spike-scratch/netns-density-295-v5c"
SSH = ["ssh", "-o", "BatchMode=yes", target]


def redact(b):
    return b.replace(target.encode(), b"<metal-target>").replace(address.encode(), b"<metal-address>")


# 1. copy only this directory (never --delete outside it; evidence/out are not pushed back)
subprocess.run(SSH + [f"mkdir -p {REMOTE}"], check=True)
subprocess.run(["rsync", "-a", "--delete", "--exclude=/out/", "--exclude=evidence/", "--exclude=target/", "--exclude=__pycache__/",
                "-e", "ssh -o BatchMode=yes", f"{SCRATCH}/", f"{target}:{REMOTE}/"], check=True)
head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
ch = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT / "vendors/cloud-hypervisor", text=True).strip()
(local / "source-manifest.json").write_text(json.dumps({"superproject_head": head, "ch_submodule_head_local": ch,
    "command": f"sudo -n python3 {REMOTE}/{attempt}/run.py {' '.join(sys.argv[2:])}"}, indent=2))

# 2. run as root
start = time.time()
cmd = f"cd {REMOTE} && sudo -n env HOME=/root PATH=/home/ubuntu/.cargo/bin:$PATH OVERDRIVE_METAL_KERNEL={env.get('OVERDRIVE_METAL_KERNEL','/srv/vm/overdrive-testing/kernel')} python3 -u {attempt}/run.py {' '.join(sys.argv[2:])}"
with (local / "native-launch.log").open("wb") as out:
    proc = subprocess.Popen(SSH + [cmd], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    for line in proc.stdout:
        line = redact(line)
        out.write(line); out.flush()
        sys.stdout.buffer.write(line); sys.stdout.buffer.flush()
    rc = proc.wait()
    out.write(json.dumps({"launcher_rc": rc, "wall_s": round(time.time() - start, 1)}).encode() + b"\n")

# 3. retrieve evidence
r = subprocess.run(SSH + [f"sudo -n tar -C {REMOTE}/{attempt} -czf - evidence"], capture_output=True)
if r.returncode == 0 and r.stdout:
    ev.mkdir()
    with tarfile.open(fileobj=io.BytesIO(r.stdout), mode="r:gz") as tar:
        for m in tar:
            if not m.isfile():
                continue
            rel = pathlib.PurePosixPath(m.name).relative_to("evidence")
            assert ".." not in rel.parts
            dst = ev / pathlib.Path(str(rel))
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_bytes(redact(tar.extractfile(m).read()))
    for f in ("source-manifest.json", "native-launch.log"):
        (ev / f).write_bytes((local / f).read_bytes())
print(f"launcher rc={rc}")
sys.exit(rc)
