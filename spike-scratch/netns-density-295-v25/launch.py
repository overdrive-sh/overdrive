#!/usr/bin/env python3
"""Launch one V-25 stage on the canonical metal host under the exclusive lease, stream the log
(target address redacted), then retrieve evidence-<stage>/. Usage: launch.py host|vm"""
import io, json, os, pathlib, subprocess, sys, tarfile, time
SCRATCH = pathlib.Path(__file__).resolve().parent
ROOT = SCRATCH.parents[1]
stage = sys.argv[1]
env = os.environ.copy()
for entry in (ROOT / ".env").read_text().splitlines():
    if "=" in entry and entry.startswith("OVERDRIVE_METAL_"):
        k, v = entry.split("=", 1); env.setdefault(k, v.strip().strip('"').strip("'"))
target = env["OVERDRIVE_METAL_TARGET"]; address = target.split("@")[-1]
env["OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS"] = env.get("V25_LEASE_WAIT", "3000")
env["RSYNC_BIN"] = str(ROOT / "spike-scratch/netns-density-295-ch-vhost-vsock/rsync-owned-filter.sh")
env["OVERDRIVE_METAL_SCENARIO"] = f"spike-netns-density-295-v25-{stage}"
local = SCRATCH / "out" / f"launch-{stage}-{int(time.time())}"; local.mkdir(parents=True)
git = lambda *a, cwd=ROOT: subprocess.check_output(["git", *a], cwd=cwd, text=True).strip()
manifest = {"superproject_head": git("rev-parse", "HEAD"), "ch_fork_head": git("rev-parse", "HEAD", cwd=ROOT / "vendors/cloud-hypervisor")}
(local / "source-manifest.json").write_text(json.dumps(manifest, indent=2)); print(manifest, flush=True)
cmd = ["cargo", "xtask", "metal", "run", "--", "python3", "spike-scratch/netns-density-295-v25/increment-a/run.py", stage]
with (local / "native-launch.log").open("w") as out:
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    for line in p.stdout:
        line = line.replace(target, "<metal-target>").replace(address, "<metal-address>")
        out.write(line); out.flush(); print(line, end="", flush=True)
    rc = p.wait()
r = subprocess.run(["ssh", "-o", "BatchMode=yes", target, f"sudo -n tar -C overdrive/spike-scratch/netns-density-295-v25/increment-a -czf - evidence/{stage}"], capture_output=True)
if r.returncode == 0:
    dst = SCRATCH / "increment-a"
    with tarfile.open(fileobj=io.BytesIO(r.stdout), mode="r:gz") as tar:
        for m in tar:
            if not m.isfile(): continue
            assert ".." not in m.name
            d = (dst / m.name); d.parent.mkdir(parents=True, exist_ok=True)
            d.write_bytes(tar.extractfile(m).read().replace(address.encode(), b"<metal-address>"))
    for f in ("source-manifest.json", "native-launch.log"):
        (dst / "evidence" / stage / f).write_bytes((local / f).read_bytes())
    print(json.dumps({"retrieved": True, "launcher_rc": rc}))
else:
    print(json.dumps({"retrieved": False, "rc": r.returncode, "stderr": r.stderr.decode().replace(address, "<addr>")[:300]}))
sys.exit(rc)
