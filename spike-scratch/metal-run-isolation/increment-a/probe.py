import fcntl, json, os, socket, tempfile
from pathlib import Path

result = {"kernel": os.uname().release, "arch": os.uname().machine}
with tempfile.TemporaryDirectory(prefix="metal-isolation-design-") as d:
    path = Path(d) / "maintenance.lock"
    one = open(path, "w")
    two = open(path, "w")
    exclusive = open(path, "w")
    fcntl.flock(one, fcntl.LOCK_SH | fcntl.LOCK_NB)
    fcntl.flock(two, fcntl.LOCK_SH | fcntl.LOCK_NB)
    result["two_shared_holders"] = True
    try:
        fcntl.flock(exclusive, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result["exclusive_refused_while_shared"] = False
    except BlockingIOError:
        result["exclusive_refused_while_shared"] = True
    one.close()
    two.close()
    fcntl.flock(exclusive, fcntl.LOCK_EX | fcntl.LOCK_NB)
    result["exclusive_after_both_release"] = True
    exclusive.close()
    slots = [open(Path(d) / f"slot-{n}.lock", "w") for n in range(2)]
    for held in slots:
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
    collision = open(Path(d) / "slot-0.lock", "w")
    try:
        fcntl.flock(collision, fcntl.LOCK_EX | fcntl.LOCK_NB)
        result["same_slot_refused"] = False
    except BlockingIOError:
        result["same_slot_refused"] = True
    collision.close()
    for held in slots:
        held.close()
listeners = [socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM) for _ in range(2)]
try:
    for listener in listeners:
        listener.bind((socket.VMADDR_CID_ANY, socket.VMADDR_PORT_ANY))
        listener.listen(1)
    ports = [listener.getsockname()[1] for listener in listeners]
    result["simultaneous_kernel_assigned_ports"] = ports
    result["distinct_nonreserved_ports"] = len(set(ports)) == 2 and min(ports) > 1023
finally:
    for listener in listeners:
        listener.close()
result["kernel_source_candidates"] = [str(p) for p in [Path("/lib/modules") / os.uname().release / "build" / "drivers/vhost/vsock.c", Path("/usr/src/linux-7.0/drivers/vhost/vsock.c"), Path("/srv/linux/drivers/vhost/vsock.c")] if p.is_file()]
print(json.dumps(result, sort_keys=True))
assert all(result[k] for k in ["two_shared_holders", "exclusive_refused_while_shared", "exclusive_after_both_release", "same_slot_refused", "distinct_nonreserved_ports"])
