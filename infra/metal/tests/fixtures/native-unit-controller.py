#!/usr/bin/env python3
"""NativeUnit owner-level fixture; public bootstrap remains the EX/waiter SUT.

The two file inputs and 20-second finalizer bound are shared with the existing
Rust NativeUnit. Records, locks and cleanup are produced only by the captured
production durable helper and actual systemd callback. Printed fixture_input
contains constructor inputs, never an observed owner-state projection.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time


reference = sys.argv[1]
if len(reference) != 24 or any(c not in "0123456789abcdef" for c in reference):
    raise ValueError("invalid reference run ID")
runs = Path("/var/lib/overdrive-metal/runs")
metadata = dict(line.split("=", 1) for line in (runs / reference / "metadata.v2").read_text().splitlines())
if metadata["format"] != "overdrive-metal-run-v2" or metadata["run_id"] != reference or metadata["unit"] != f"overdrive-metal-run-{reference}.service":
    raise ValueError("reference owner identity mismatch")
source = Path(base64.b64decode(metadata["remote_dir_b64"], validate=True).decode())
workspace = base64.b64decode(metadata["workspace_b64"], validate=True).decode()
actor = Path(tempfile.mkdtemp(prefix="metal-admission-owner-", dir="/tmp"))
run_id = hashlib.sha256(os.fsencode(actor)).hexdigest()[:24]
captured = actor / "captured-holder.sh"
primitive = actor / "native-unit"
shutil.copyfile(source / "infra/metal/lease-holder.sh", captured)
shutil.copyfile(source / "infra/metal/tests/fixtures/native-unit", primitive)
barrier = actor / "maintenance.lock"
barrier_owner = actor / "maintenance.owner"
tree = actor / "tree.lock"
tree_owner = actor / "tree.owner"
gate = actor / "finalizer"
gate.write_text("#!/usr/bin/env bash\nexec bash " + " ".join(shlex.quote(str(arg)) for arg in
    (primitive, "finalize", actor, os.getpid(), captured, run_id, runs)) + "\n")
gate.chmod(0o700)
body = "bash " + " ".join(shlex.quote(str(arg)) for arg in (primitive, "command", actor))
# Concrete real filesystem inputs for the finalizer's exact three-root promise.
roots = [Path("/srv/vm/metal", run_id), Path("/run/ovd-gk", run_id), source / "target/metal-run-tmp" / run_id]
for index, root in enumerate(roots):
    root.mkdir()
    if index < 2:
        root.chmod(0o710)
        os.chown(root, 0, 4200)
    else:
        root.chmod(0o700)
    (root / "fixture-owned-input").write_bytes(b"controlled prior owner\n")
command = ["/usr/bin/systemd-run", "--quiet", "--pipe", "--wait", "--collect", "--expand-environment=no",
           "--property=Type=exec", "--property=KillMode=mixed", "--property=RuntimeMaxSec=60s", "--property=TimeoutStopSec=45s",
           f"--unit=overdrive-metal-run-{run_id}.service", f"--property=ExecStopPost={gate}",
           "bash", str(captured), "--durable", str(barrier), str(barrier_owner), "120", run_id,
           "metal-protocol-conformance", workspace, metadata["commit"], metadata["source_digest"], str(source),
           "root", "privileged", str(source.parent), os.environ["PATH"], body, str(runs), str(tree), str(tree_owner), "ordinary"]
with (actor / "controller.stdout").open("wb") as stdout, (actor / "controller.stderr").open("wb") as stderr:
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=stdout, stderr=stderr)
    print(json.dumps({"fixture_input": {"run_id": run_id, "actor": str(actor), "barrier": str(barrier),
          "barrier_owner": str(barrier_owner), "tree": str(tree), "tree_owner": str(tree_owner)}}), flush=True)
    try:
        deadline = time.monotonic() + 7
        phase = runs / run_id / "phase.v1"
        while not (phase.is_file() and "phase=preparing\n" in phase.read_text()):
            if child.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError(f"actual fixture owner {run_id} did not reach preparation; diagnostics: {actor}")
            time.sleep(0.01)
        child.stdin.write(b"START\n")
        child.stdin.flush()
        status = child.wait()
    finally:
        # Same NativeUnit file inputs; never remove a barrier inode while its
        # public EX/waiter participants can still be using it. The driver owns
        # final fixture-directory removal after their actual stopped snapshots.
        (actor / "command.exit").write_text("0\n")
        (actor / "stop").write_text("release\n")
        child.stdin.close()
        if child.poll() is None:
            subprocess.run(["/usr/bin/systemctl", "stop", f"overdrive-metal-run-{run_id}.service"], check=True)
            child.wait()
print(json.dumps({"fixture_result": {"status": status,
      "controller_stdout_b64": base64.b64encode((actor / "controller.stdout").read_bytes()).decode(),
      "controller_stderr_b64": base64.b64encode((actor / "controller.stderr").read_bytes()).decode()}}), flush=True)
sys.exit(status)
