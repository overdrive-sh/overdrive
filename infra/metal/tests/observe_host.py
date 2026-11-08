#!/usr/bin/env python3
"""Read-only tooling-test observer; no guest, runner, or protocol implementation.

Run on the actual Metal host through the external acceptance driver. Model
history/ghosts are deliberately absent. A missing observation raises an error.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import sys


def command(argv):
    return subprocess.run(argv, check=True, capture_output=True, text=True).stdout


def physical():
    nft = json.loads(command(["nft", "-j", "list", "ruleset"]))
    nft["nftables"] = [row for row in nft.get("nftables", []) if "metainfo" not in row]
    for row in nft["nftables"]:
        for expression in row.get("rule", {}).get("expr", []):
            if "counter" in expression:
                expression["counter"] = {}
    programs = json.loads(command(["bpftool", "-j", "prog", "show"]))
    cgroup = Path("/sys/fs/cgroup/overdrive.slice/workloads.slice")
    return {
        "modules": {line.split()[0]: line.split()[1] for line in Path("/proc/modules").read_text().splitlines()},
        "nft": nft,
        "bpf_programs": sorted((row["id"], row["type"], row.get("name"), row.get("tag")) for row in programs),
        "routes": json.loads(command(["ip", "-j", "route", "show", "table", "all"])),
        "test_cgroups": sorted(str(path.relative_to(cgroup)) for path in cgroup.rglob("*") if path.is_dir()) if cgroup.exists() else None,
        "initial_vsock_modes": {name: Path("/proc/sys/net/vsock", name).read_text() for name in ("ns_mode", "child_ns_mode")},
    }


def record(path):
    raw = path.read_bytes()
    if not raw.endswith(b"\n"):
        raise ValueError(f"unterminated {path}")
    result = {}
    for line in raw.decode().splitlines():
        key, value = line.split("=", 1)
        if key in result:
            raise ValueError(f"duplicate {key} in {path}")
        result[key] = value
    return result


def owner(run_id):
    if len(run_id) != 24 or any(char not in "0123456789abcdef" for char in run_id):
        raise ValueError("invalid test input run ID")
    root = Path("/var/lib/overdrive-metal/runs", run_id)
    metadata = record(root / "metadata.v2")
    if metadata["format"] != "overdrive-metal-run-v2" or metadata["run_id"] != run_id:
        raise ValueError("v2 filename/format/identity mismatch")
    import base64
    source = Path(base64.b64decode(metadata["remote_dir_b64"], validate=True).decode())
    result = {"metadata": metadata, "checkout": record(root / "checkout.v1") if (root / "checkout.v1").exists() else None}
    for name in ("owner.v1", "phase.v1", "terminal.v1", "completion.v1"):
        if (root / name).exists():
            result[name] = record(root / name)
    unit = f"overdrive-metal-run-{run_id}.service"
    facts = command(["systemctl", "show", unit, "--property=MainPID,ActiveState,ControlGroup"])
    unit_facts = dict(line.split("=", 1) for line in facts.splitlines())
    result["unit"] = unit_facts
    pids = set()
    cgroup_path = Path("/sys/fs/cgroup", unit_facts.get("ControlGroup", "").lstrip("/"))
    if unit_facts.get("ControlGroup") and cgroup_path.exists():
        for path in [cgroup_path / "cgroup.procs", *cgroup_path.rglob("cgroup.procs")]:
            if path.exists():
                pids.update(int(pid) for pid in path.read_text().split())
    # VMM confinement can move a launcher below its own cgroup. Include actual
    # processes holding this invocation's exact image/runtime paths as well as
    # the durable unit's process complement; never infer ownership from a PID list
    # maintained by the driver or a broad process-name match.
    owned_roots=(str(Path("/srv/vm/metal",run_id)),str(Path("/run/ovd-gk",run_id)),str(source / "target/metal-run-tmp" / run_id))
    def owned_path(value):
        return any(value==root or root+"/" in value for root in owned_roots)
    for proc in Path("/proc").iterdir():
        if not proc.name.isdecimal():
            continue
        try:
            argv=(proc/"cmdline").read_bytes().decode(errors="replace").split("\0")
            if not any(owned_path(arg) for arg in argv):
                continue
            descriptors=[]
            for fd in (proc/"fd").iterdir():
                try: descriptors.append(os.readlink(fd))
                except FileNotFoundError: continue
            executable=Path(argv[0]).name if argv else ""
            if executable=="cloud-hypervisor" or any(owned_path(fd) or "vhost-vsock" in fd for fd in descriptors):
                pids.add(int(proc.name))
        except (FileNotFoundError,ProcessLookupError):
            continue
    result["processes"] = {}
    for pid in sorted(pids):
        proc = Path("/proc", str(pid))
        try:
            fds = {}
            for fd in (proc / "fd").iterdir():
                try:
                    target = os.readlink(fd)
                    info = (proc / "fdinfo" / fd.name).read_text()
                    if "lock:" in info or "vhost-vsock" in target or "kvm" in target:
                        fds[fd.name] = {"target": target, "fdinfo": info}
                except FileNotFoundError:
                    continue
            result["processes"][str(pid)] = {"argv": (proc / "cmdline").read_bytes().decode(errors="replace").split("\0"), "netns": os.readlink(proc / "ns/net"), "fds": fds}
        except FileNotFoundError:
            continue
    result["roots"] = {}
    result["launcher_bases"] = {}
    for name,path in {"guest_staging":Path("/srv/vm/metal"),"vmm_runtime":Path("/run/ovd-gk")}.items():
        try:
            item=path.lstat()
            result["launcher_bases"][name]={"exists":True,"uid":item.st_uid,"gid":item.st_gid,"mode":stat.S_IMODE(item.st_mode),"symlink":stat.S_ISLNK(item.st_mode)}
        except FileNotFoundError:
            result["launcher_bases"][name]={"exists":False}
    for name, path in {
        "guest_staging": Path("/srv/vm/metal", run_id),
        "vmm_runtime": Path("/run/ovd-gk", run_id),
        "command_tmp": source / "target/metal-run-tmp" / run_id,
    }.items():
        try:
            metadata = path.lstat()
            result["roots"][name] = {"exists": True, "uid": metadata.st_uid, "gid": metadata.st_gid, "mode": stat.S_IMODE(metadata.st_mode), "symlink": stat.S_ISLNK(metadata.st_mode)}
        except FileNotFoundError:
            result["roots"][name] = {"exists": False}
    result["source_marker"] = record(source / ".overdrive-metal-source")
    return result


print(json.dumps({"physical": physical(), "owners": {run_id: owner(run_id) for run_id in sys.argv[1:]}}, sort_keys=True))
