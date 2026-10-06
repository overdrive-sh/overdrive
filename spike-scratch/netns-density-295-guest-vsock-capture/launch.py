#!/usr/bin/env python3
"""Launch one increment on the canonical metal host under the exclusive lease,
stream its log (target address redacted), then retrieve its evidence.

Usage: python3 spike-scratch/netns-density-295-guest-vsock-capture/launch.py increment-a
"""
import hashlib, io, json, os, pathlib, subprocess, sys, tarfile, time

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
env["OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS"] = "120"
env["RSYNC_BIN"] = str(SCRATCH / "rsync-owned-filter.sh")
env["OVERDRIVE_METAL_SCENARIO"] = f"spike-netns-density-295-guest-vsock-capture-{attempt}"


def git(*a, cwd=ROOT):
    return subprocess.check_output(["git", *a], cwd=cwd, text=True).strip()


manifest = {
    "superproject_head": git("rev-parse", "HEAD"),
    "ch_submodule_head": git("rev-parse", "HEAD", cwd=ROOT / "vendors/cloud-hypervisor"),
    "ch_submodule_branch": git("rev-parse", "--abbrev-ref", "HEAD", cwd=ROOT / "vendors/cloud-hypervisor"),
    "ch_submodule_dirty": git("status", "--porcelain", cwd=ROOT / "vendors/cloud-hypervisor"),
    "ch_commits_since_v53": git("log", "--oneline", "v53.0..HEAD", cwd=ROOT / "vendors/cloud-hypervisor"),
    "spike_sources": {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in SCRATCH.rglob("*")
        if p.is_file() and "evidence" not in p.parts and "out" not in p.parts and "__pycache__" not in p.parts
    },
    "command": f"cargo xtask metal run -- python3 {(inc / 'run.py').relative_to(ROOT)} {' '.join(sys.argv[2:])}",
}
(local / "source-manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True))
print(json.dumps({k: v for k, v in manifest.items() if k != "spike_sources"}, indent=2), flush=True)

start = time.time()
with (local / "native-launch.log").open("w") as out:
    proc = subprocess.Popen(
        ["cargo", "xtask", "metal", "run", "--", "python3", str((inc / "run.py").relative_to(ROOT)), *sys.argv[2:]],
        cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    for line in proc.stdout:
        line = line.replace(target, "<metal-target>").replace(address, "<metal-address>")
        out.write(line)
        out.flush()
        print(line, end="", flush=True)
    rc = proc.wait()
    out.write(json.dumps({"launcher_rc": rc, "wall_s": round(time.time() - start, 1)}) + "\n")

r = subprocess.run(["ssh", "-o", "BatchMode=yes", target,
                    f"sudo -n tar -C overdrive/spike-scratch/netns-density-295-guest-vsock-capture/{attempt} -czf - evidence"],
                   capture_output=True)
if r.returncode == 0:
    ev.mkdir()
    with tarfile.open(fileobj=io.BytesIO(r.stdout), mode="r:gz") as tar:
        for m in tar:
            if not m.isfile():
                continue
            rel = pathlib.PurePosixPath(m.name).relative_to("evidence")
            assert ".." not in rel.parts
            data = tar.extractfile(m).read().replace(address.encode(), b"<metal-address>")
            dst = ev / pathlib.Path(str(rel))
            dst.parent.mkdir(parents=True, exist_ok=True)
            dst.write_bytes(data)
    for f in ("source-manifest.json", "native-launch.log"):
        (ev / f).write_bytes((local / f).read_bytes())
    print(json.dumps({"retrieved": True, "launcher_rc": rc, "archive_sha256": hashlib.sha256(r.stdout).hexdigest()}))
else:
    print(json.dumps({"retrieved": False, "rc": r.returncode, "stderr": r.stderr.decode().replace(address, "<addr>")}))
sys.exit(rc)
