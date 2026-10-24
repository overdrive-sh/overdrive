"""Copy retained console bytes before the next canonical Metal --delete sync.

This transfers evidence and fixes ownership only in this review's own probe
increment. It executes no test, kernel/module operation or source change.
Native executions themselves use cargo xtask metal run and its full preflight.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

increment, run = sys.argv[1:]
assert increment in ("increment-c", "increment-d")
assert "/" not in run and run.startswith("default-kit-")
target = os.environ.get("OVERDRIVE_METAL_TARGET")
if not target:
    for line in Path(".env").read_text().splitlines():
        if line.startswith("OVERDRIVE_METAL_TARGET="):
            target = line.split("=", 1)[1].strip().strip("\"'")
assert target
relative = f"spike-scratch/netns-density-295-review-11-06/{increment}"
remote = r'''
import base64,hashlib,json,os,pathlib,sys
r=pathlib.Path.cwd()
p=r/sys.argv[1]
c=p/"evidence/reruns"/sys.argv[2]/"guest-console/native-console.txt"
sources=["crates/overdrive-bpf/src/programs/guest_capture.rs","crates/overdrive-init/src/guest_flow/kernel_io.rs","crates/overdrive-init/src/guest_flow/owner.rs","crates/overdrive-init/src/transport/kernel.rs","crates/overdrive-testing/src/guest_kit.rs","crates/overdrive-testing/bin/guest_peer.rs"]
artifacts=["target/x86_64-unknown-linux-musl/release/overdrive-init","target/x86_64-unknown-linux-musl/release/overdrive-guest-peer","target/bpf/overdrive_guest_bpf.o","target/bpf/overdrive_guest_tcp_correlation_candidate_bpf.o","target/kmod/7.0.0-38-generic/ovd_fwd.ko"]
d={"console_base64":base64.b64encode(c.read_bytes()).decode(),"console_sha256":hashlib.sha256(c.read_bytes()).hexdigest(),"source_receipt":(r/".overdrive-metal-source").read_text(),"kernel":os.uname().release,"sources":{f:hashlib.sha256((r/f).read_bytes()).hexdigest() for f in sources},"artifacts":{f:hashlib.sha256((r/f).read_bytes()).hexdigest() for f in artifacts},"ownership_before":{"uid":c.stat().st_uid,"gid":c.stat().st_gid}}
lock=p/"Cargo.lock"
if lock.exists():d["cargo_lock_base64"]=base64.b64encode(lock.read_bytes()).decode()
u,g=r.stat().st_uid,r.stat().st_gid
for f in [p,*p.rglob("*")]:os.chown(f,u,g,follow_symlinks=False)
d["ownership_after"]={"uid":c.stat().st_uid,"gid":c.stat().st_gid}
print(json.dumps(d))
'''
# The same lock serializes this receipt transfer with canonical native runs.
command = "cd overdrive && "+shlex.join(["sudo", "-n", "flock", "-x", "-w", "30",
                      "/run/lock/overdrive-metal-shared.lock", "python3", "-c", remote, relative, run])
capture = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10",
                          target, command], capture_output=True, text=True, check=True)
data = json.loads(capture.stdout)
console = base64.b64decode(data.pop("console_base64"))
assert hashlib.sha256(console).hexdigest() == data["console_sha256"]
directory = Path(relative)/"evidence/reruns"/run
(directory/"guest-console").mkdir(parents=True, exist_ok=True)
(directory/"guest-console/native-console.txt").write_bytes(console)
if "cargo_lock_base64" in data:
    (Path(relative)/"Cargo.lock").write_bytes(base64.b64decode(data.pop("cargo_lock_base64")))
(directory/"identity-and-retention.json").write_text(json.dumps(data, indent=2)+"\n")
(directory/"fetch-command.txt").write_text(command+"\n")
print(json.dumps({"increment":increment,"console_bytes":len(console),
                  "console_sha256":data["console_sha256"],"kernel":data["kernel"],
                  "artifacts":data["artifacts"],"ownership_after":data["ownership_after"]},indent=2))
