#!/usr/bin/env bash
# The mandatory per-test nextest adapter for ordinary Metal runs.
set -euo pipefail

test_error() {
  local code="$1"
  local status="$2"
  local context="${3:-}"
  printf 'OVERDRIVE_METAL_TEST_ERROR error=%s' "$code" >&2
  [ -z "$context" ] || printf ' %s' "$context" >&2
  printf '\n' >&2
  exit "$status"
}

invalid_context() {
  test_error invalid_run_context 78 "field=$1"
}

# The placement resolver, shared by an ordinary run (`run`) and the eager check
# (`check`). A test is placed by, first match wins: its own record; a
# `module_host_pure` record for exactly its module; the binary's
# `default_host_pure`; otherwise it is unplaced. A record is a sequence of NUL-terminated fields `<kind>`,
# `<test>`, `<argc>`, then `argc` arguments, in the `overdrive_test_placement`
# section of the test binary; `<test>` and a module record's path start at the
# crate segment, which is dropped to compare with nextest's names.
#
# Exit status of `run`: 0 placed (prints `kind US group US fixture`), 12
# unplaced, 13 invalid (prints the detail), 15 conflict.
read -r -d '' PLACEMENT_PY <<'PY' || true
import json
import struct
import sys

GROUPS = {
    "probe", "core", "unload", "ulp", "mtls-relay", "cgroup-default",
    "stress", "nested-boundary",
}
SECTION = b"overdrive_test_placement"
ARGUMENT_COUNT = {
    "host_pure": 0, "native_launcher": 0, "module_host_pure": 0,
    "default_host_pure": 0, "guest": 3, "lane_operator": 1, "lane_owner": 1,
}
PER_TEST = {"host_pure", "native_launcher", "guest", "lane_operator", "lane_owner"}
NOT_HOST_PURE = {"native_launcher", "guest", "lane_operator", "lane_owner"}
# What the wrapper does for a record: the host-pure kinds run the original
# chain; a guest or lane test boots its own guest from its host half, in the
# local-mode vsock namespace.
RECORD_KIND = {
    "host_pure": "host-pure", "module_host_pure": "host-pure",
    "default_host_pure": "host-pure", "native_launcher": "native-launcher",
    "guest": "guest-declared", "lane_operator": "guest-declared",
    "lane_owner": "guest-declared",
}

class Invalid(Exception):
    pass

class Unclassified(Exception):
    pass

def section_bytes(path):
    # 64-bit little-endian ELF only: the test binaries of the Metal host.
    try:
        binary = open(path, "rb")
    except OSError:
        raise Invalid("binary=unreadable")
    with binary:
        header = binary.read(64)
        if len(header) < 64 or header[:4] != b"\x7fELF" or header[4] != 2 or header[5] != 1:
            raise Invalid("binary=not-elf")
        (section_table, entry_size, entry_count, names_index) = (
            struct.unpack_from("<Q", header, 40)[0],
            struct.unpack_from("<H", header, 58)[0],
            struct.unpack_from("<H", header, 60)[0],
            struct.unpack_from("<H", header, 62)[0],
        )
        if section_table == 0 or entry_size < 64 or names_index >= entry_count:
            return None
        binary.seek(section_table)
        table = binary.read(entry_size * entry_count)
        def entry(index):
            name, _kind, _flags, _addr, offset, size = struct.unpack_from(
                "<IIQQQQ", table, index * entry_size
            )
            return name, offset, size
        _, names_offset, names_size = entry(names_index)
        binary.seek(names_offset)
        names = binary.read(names_size)
        for index in range(entry_count):
            name, offset, size = entry(index)
            end = names.find(b"\0", name)
            if names[name:end] == SECTION:
                binary.seek(offset)
                return binary.read(size)
    return None

def read_records(path):
    data = section_bytes(path)
    if not data:
        return []
    fields = data.split(b"\0")
    if fields.pop() != b"":
        raise Invalid("record=malformed")
    try:
        fields = [field.decode() for field in fields]
    except UnicodeDecodeError:
        raise Invalid("record=malformed")
    records = []
    index = 0
    while index < len(fields):
        if index + 3 > len(fields):
            raise Invalid("record=malformed")
        kind, test, argc = fields[index:index + 3]
        if kind not in ARGUMENT_COUNT:
            raise Invalid("kind=" + kind)
        if argc != str(ARGUMENT_COUNT[kind]):
            raise Invalid("record=malformed")
        count = ARGUMENT_COUNT[kind]
        arguments = fields[index + 3:index + 3 + count]
        if len(arguments) != count or (test == "") != (kind == "default_host_pure"):
            raise Invalid("record=malformed")
        if kind == "guest":
            if arguments[0] not in GROUPS:
                raise Invalid("group=" + arguments[0])
            if arguments[2] not in {"0", "1"}:
                raise Invalid("record=malformed")
        records.append((kind, test, arguments))
        index += 3 + count
    return records

def drop_crate(path):
    return path.split("::", 1)[1] if "::" in path else ""

def has_conflict(records):
    kinds = {kind for kind, _test, _arguments in records}
    return "default_host_pure" in kinds and bool(kinds & NOT_HOST_PURE)

def resolve_record(records, test_name):
    for kind, test, arguments in records:
        if kind in PER_TEST and drop_crate(test) == test_name:
            return kind, arguments
    module = test_name.rpartition("::")[0]
    for kind, test, _arguments in records:
        if kind == "module_host_pure" and drop_crate(test) == module:
            return kind, []
    for kind, _test, _arguments in records:
        if kind == "default_host_pure":
            return kind, []
    return None

def record_placement(kind):
    return RECORD_KIND[kind]

def run(binary_path, test_name):
    try:
        records = read_records(binary_path)
        if has_conflict(records):
            raise SystemExit(15)
        found = resolve_record(records, test_name)
        if found is None:
            raise Unclassified()
        placement = record_placement(found[0])
    except Unclassified:
        raise SystemExit(12)
    except Invalid as problem:
        print(problem)
        raise SystemExit(13)
    print(placement)

def check():
    listing = json.load(sys.stdin)
    findings = False
    invalid = False
    for suite in listing.get("rust-suites", {}).values():
        binary_id = suite["binary-id"]
        try:
            records = read_records(suite["binary-path"])
        except Invalid as problem:
            print("placement_invalid", binary_id, problem, file=sys.stderr)
            invalid = True
            continue
        if has_conflict(records):
            print("conflict", binary_id)
            findings = True
            continue
        for test_name, case in suite.get("testcases", {}).items():
            if resolve_record(records, test_name) is not None:
                continue
            print("unplaced", binary_id, test_name)
            findings = True
    raise SystemExit(78 if invalid else 1 if findings else 0)

if sys.argv[1] == "run":
    run(*sys.argv[2:4])
else:
    check()
PY

# `--check-placement` reads `nextest list --message-format json` on stdin and
# reports the tests no record places (`unplaced <binary-id>
# <test>`) and the binaries with a default beside a non-host-pure record
# (`conflict <binary-id>`). It needs no Metal run context.
if [ "${1:-}" = --check-placement ]; then
  exec python3 -c "$PLACEMENT_PY" check
fi

# No Metal context means nextest has no reason to alter the configured runner.
if [ -z "${OVERDRIVE_METAL_RUN_ID:-}" ] && \
  [ -z "${OVERDRIVE_METAL_EXECUTION_CLASS:-}" ] && \
  [ -z "${OVERDRIVE_METAL_SOURCE_ROOT:-}" ]; then
  exec "$@"
fi

# Explicit maintenance preserves the caller's complete exec chain.
[ "${OVERDRIVE_METAL_EXECUTION_CLASS:-}" = exclusive ] && exec "$@"
[ "${OVERDRIVE_METAL_EXECUTION_CLASS:-}" = ordinary ] || invalid_context execution_class
[[ "${OVERDRIVE_METAL_RUN_ID:-}" =~ ^[0-9a-f]{24}$ ]] || invalid_context run_id

SOURCE_ROOT="${OVERDRIVE_METAL_SOURCE_ROOT:-}"
[ -n "$SOURCE_ROOT" ] || invalid_context source_owner
SOURCE_CANONICAL="$(realpath -e -- "$SOURCE_ROOT" 2>/dev/null)" || invalid_context source_owner
[ "$SOURCE_CANONICAL" = "$SOURCE_ROOT" ] || invalid_context source_owner
[ -d "$SOURCE_ROOT" ] && [ ! -L "$SOURCE_ROOT" ] || invalid_context source_owner
for owner_file in "$SOURCE_ROOT/.overdrive-metal-checkout.v1" "$SOURCE_ROOT/.overdrive-metal-source"; do
  [ -f "$owner_file" ] && [ ! -L "$owner_file" ] || invalid_context source_owner
done
WORKSPACE_ROOT="${NEXTEST_WORKSPACE_ROOT:-$PWD}"
[ -n "$WORKSPACE_ROOT" ] || invalid_context source_owner
WORKSPACE_CANONICAL="$(realpath -e -- "$WORKSPACE_ROOT" 2>/dev/null)" || invalid_context source_owner
EXPECTED_TMP_ROOT="$SOURCE_ROOT/target/metal-run-tmp/$OVERDRIVE_METAL_RUN_ID"
[ -n "${TMPDIR:-}" ] && [ -d "$TMPDIR" ] && [ ! -L "$TMPDIR" ] || invalid_context source_owner
TMP_ROOT_CANONICAL="$(realpath -e -- "$TMPDIR" 2>/dev/null)" || invalid_context source_owner
[ "$TMP_ROOT_CANONICAL" = "$EXPECTED_TMP_ROOT" ] || invalid_context source_owner
case "$WORKSPACE_CANONICAL" in
  "$SOURCE_ROOT"|"$SOURCE_ROOT"/*|"$TMP_ROOT_CANONICAL"|"$TMP_ROOT_CANONICAL"/*) ;;
  *) invalid_context source_owner ;;
esac

[ "$#" -gt 0 ] || test_error test_unclassified 78
BINARY_ID="${NEXTEST_BINARY_ID:-}"
TEST_NAME="${NEXTEST_TEST_NAME:-}"
[ -n "$BINARY_ID" ] && [ -n "$TEST_NAME" ] || test_error test_unclassified 78

# The test binary carries the placement records its declarations wrote; a test
# no record places is refused.
set +e
PLACEMENT="$(python3 -c "$PLACEMENT_PY" run "$1" "$TEST_NAME")"
PARSER_STATUS=$?
set -e
case "$PARSER_STATUS" in
  0) ;;
  12) test_error test_unclassified 78 ;;
  15) test_error placement_conflict 78 "binary=$BINARY_ID" ;;
  *) test_error placement_invalid 78 "$PLACEMENT" ;;
esac
PLACEMENT_KIND="$PLACEMENT"

# Isolation profiles for the per-run namespace set.
#   net  - a private network namespace in vsock "local" mode (own nft tables,
#          routes, rules, interfaces, loopback listeners and vsock CID space).
#   node - `net` plus everything a `serve` boot owns on a node: a private
#          mount namespace with its own bpffs instance, its own tmpfs over
#          /run/overdrive and /run/netns, and a cgroup namespace whose root is
#          a per-run cgroup, mounted over /sys/fs/cgroup, so `overdrive.slice`
#          is per-run. Boot-time reclamation therefore sees only this run.
run_isolated_namespace() {
  exec python3 - "$@" <<'PY'
import ctypes
import fcntl
import os
import signal
import socket
import struct
import sys
import time

CLONE_NEWNS = 0x00020000
CLONE_NEWCGROUP = 0x02000000
CLONE_NEWNET = 0x40000000
MS_NOSUID = 0x2
MS_NODEV = 0x4
MS_NOEXEC = 0x8
MS_REC = 0x4000
MS_PRIVATE = 0x40000
PR_SET_PDEATHSIG = 1
MNT_DETACH = 2

CGROUP_ROOT = "/sys/fs/cgroup"
RUNS_DIR = CGROUP_ROOT + "/overdrive-metal-runs"
CONTROLLERS = ("cpu", "io", "memory", "pids")

libc = ctypes.CDLL(None, use_errno=True)
libc.unshare.argtypes = [ctypes.c_int]
libc.unshare.restype = ctypes.c_int
libc.mount.argtypes = [
    ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_ulong, ctypes.c_char_p,
]
libc.mount.restype = ctypes.c_int
libc.umount2.argtypes = [ctypes.c_char_p, ctypes.c_int]
libc.umount2.restype = ctypes.c_int
libc.prctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
libc.prctl.restype = ctypes.c_int

def fail(step, error_number=None):
    value = "none" if error_number is None else str(error_number)
    print(
        f"OVERDRIVE_METAL_TEST_ERROR error=namespace_setup_failed step={step} errno={value}",
        file=sys.stderr,
    )
    raise SystemExit(70)

def unshare(step, flags):
    if libc.unshare(flags) != 0:
        fail(step, ctypes.get_errno())

def mount(step, source, target, fstype, flags, data=None):
    if libc.mount(source, target, fstype, flags, data) != 0:
        fail(step, ctypes.get_errno())

def write_file(path, text):
    descriptor = os.open(path, os.O_WRONLY | os.O_CLOEXEC)
    try:
        os.write(descriptor, text.encode())
    finally:
        os.close(descriptor)

def read_file(path):
    with open(path, encoding="utf-8") as source:
        return source.read()

def start_time(pid):
    try:
        stat = read_file(f"/proc/{pid}/stat")
    except OSError:
        return None
    return stat[stat.rindex(")") + 2:].split()[19]

# ---- per-run cgroup ----------------------------------------------------

def prepare_runs_dir():
    os.makedirs(RUNS_DIR, exist_ok=True)
    available = read_file(CGROUP_ROOT + "/cgroup.controllers").split()
    enabled = read_file(CGROUP_ROOT + "/cgroup.subtree_control").split()
    missing = ["+" + name for name in CONTROLLERS if name in available and name not in enabled]
    if missing:
        write_file(CGROUP_ROOT + "/cgroup.subtree_control", " ".join(missing))
    runs_available = read_file(RUNS_DIR + "/cgroup.controllers").split()
    runs_enabled = read_file(RUNS_DIR + "/cgroup.subtree_control").split()
    runs_missing = ["+" + name for name in runs_available if name not in runs_enabled]
    if runs_missing:
        write_file(RUNS_DIR + "/cgroup.subtree_control", " ".join(runs_missing))

def remove_cgroup_tree(root):
    try:
        write_file(root + "/cgroup.kill", "1")
    except OSError:
        pass
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            if "populated 0" in read_file(root + "/cgroup.events"):
                break
        except OSError:
            break
        time.sleep(0.05)
    for _ in range(50):
        for directory, _names, _files in os.walk(root, topdown=False):
            try:
                os.rmdir(directory)
            except OSError:
                pass
        if not os.path.isdir(root):
            return
        time.sleep(0.1)

def sweep_dead_runs():
    # A run whose wrapper was SIGKILLed leaves its (empty or orphaned) cgroup;
    # reclaim every per-run cgroup whose owner process no longer exists.
    for name in os.listdir(RUNS_DIR):
        parts = name.rsplit("-", 2)
        if len(parts) != 3 or not parts[1].isdigit():
            continue
        if start_time(parts[1]) == parts[2]:
            continue
        remove_cgroup_tree(RUNS_DIR + "/" + name)

# ---- namespace setup (runs in the child, before exec) --------------------

def isolate_network():
    unshare("parent", CLONE_NEWNET)
    try:
        write_file("/proc/sys/net/vsock/child_ns_mode", "local\n")
    except OSError as error:
        fail("child_mode", error.errno)

    unshare("local", CLONE_NEWNET)
    try:
        mode = read_file("/proc/sys/net/vsock/ns_mode").strip()
    except OSError as error:
        fail("verify", error.errno)
    if mode != "local":
        fail("verify")

    # A fresh network namespace starts with `lo` down; the tests this namespace
    # hosts bind and connect on loopback (the forwarder probe's TCP pair, the
    # gateway's 127/8 listener), so bring it up before the test body runs.
    try:
        control = socket.socket(socket.AF_INET, socket.SOCK_DGRAM | socket.SOCK_CLOEXEC)
        request = struct.pack("16sH14x", b"lo", 0)
        flags = struct.unpack("16sH14x", fcntl.ioctl(control, 0x8913, request))[1]
        fcntl.ioctl(control, 0x8914, struct.pack("16sH14x", b"lo", flags | 0x1))
        control.close()
    except OSError as error:
        fail("loopback", error.errno)

def isolate_node(run_cgroup):
    try:
        prepare_runs_dir()
        sweep_dead_runs()
        os.mkdir(run_cgroup)
    except OSError as error:
        fail("cgroup_prepare", error.errno)
    # Join the per-run cgroup first: the cgroup namespace root is the cgroup the
    # process occupies when it unshares.
    try:
        write_file(run_cgroup + "/cgroup.procs", f"{os.getpid()}\n")
    except OSError as error:
        fail("cgroup_join", error.errno)
    unshare("node", CLONE_NEWNS | CLONE_NEWCGROUP)
    mount("mount_private", b"none", b"/", None, MS_REC | MS_PRIVATE)
    private = MS_NOSUID | MS_NODEV | MS_NOEXEC
    try:
        os.makedirs("/run/overdrive", exist_ok=True)
        os.makedirs("/run/netns", exist_ok=True)
    except OSError as error:
        fail("run_dirs", error.errno)
    mount("run_overdrive", b"tmpfs", b"/run/overdrive", b"tmpfs", private, b"mode=0755")
    mount("run_netns", b"tmpfs", b"/run/netns", b"tmpfs", private, b"mode=0755")
    # A fresh cgroup2 mount inside a non-init cgroup namespace is rooted at the
    # namespace root, i.e. the per-run cgroup.
    # The inherited mount is the host-rooted view of the same hierarchy;
    # mounting over it is refused (EBUSY), so detach it first.
    if libc.umount2(CGROUP_ROOT.encode(), MNT_DETACH) != 0:
        fail("cgroup2_detach", ctypes.get_errno())
    mount("cgroup2", b"cgroup2", CGROUP_ROOT.encode(), b"cgroup2", private)
    # bpffs is `get_tree_nodev`: each mount is its own instance with its own pins.
    mount("bpffs", b"bpf", b"/sys/fs/bpf", b"bpf", private, b"mode=0700")
    # The namespace root keeps no processes so it may delegate controllers: the
    # test process lives in a leaf below it.
    try:
        os.makedirs(CGROUP_ROOT + "/leaf", exist_ok=True)
        write_file(CGROUP_ROOT + "/leaf/cgroup.procs", f"{os.getpid()}\n")
        available = read_file(CGROUP_ROOT + "/cgroup.controllers").split()
        wanted = ["+" + name for name in CONTROLLERS if name in available]
        if wanted:
            write_file(CGROUP_ROOT + "/cgroup.subtree_control", " ".join(wanted))
        # The node's delegated subtree exists before serve boots, as it does on
        # a host that has run before: overdrive.slice/workloads.slice with the
        # controllers delegated. serve's own creation of them is idempotent.
        for relative in ("overdrive.slice", "overdrive.slice/workloads.slice"):
            directory = CGROUP_ROOT + "/" + relative
            os.makedirs(directory, exist_ok=True)
            if wanted:
                write_file(directory + "/cgroup.subtree_control", " ".join(wanted))
    except OSError as error:
        fail("cgroup_delegate", error.errno)

def run_child(argv, run_cgroup):
    libc.prctl(PR_SET_PDEATHSIG, signal.SIGKILL, 0, 0, 0)
    isolate_network()
    if run_cgroup is not None:
        isolate_node(run_cgroup)
    os.execvpe(argv[0], argv, os.environ)

def main():
    profile = sys.argv[1]
    argv = sys.argv[2:]
    if profile not in ("net", "node") or not argv:
        fail("verify")
    if profile == "net":
        # Original behaviour: the namespaces are applied in place and the
        # command replaces this process.
        isolate_network()
        os.execvpe(argv[0], argv, os.environ)

    run_id = os.environ.get("OVERDRIVE_METAL_RUN_ID", "norun")
    run_cgroup = f"{RUNS_DIR}/{run_id}-{os.getpid()}-{start_time(os.getpid())}"

    child = os.fork()
    if child == 0:
        try:
            run_child(argv, run_cgroup)
        except OSError as error:
            print(
                f"OVERDRIVE_METAL_TEST_ERROR error=namespace_setup_failed step=exec errno={error.errno}",
                file=sys.stderr,
            )
        os._exit(70)

    def forward(number, _frame):
        try:
            os.kill(child, number)
        except OSError:
            pass

    for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGQUIT):
        signal.signal(number, forward)
    while True:
        try:
            _, status = os.waitpid(child, 0)
            break
        except InterruptedError:
            continue
    remove_cgroup_tree(run_cgroup)
    if os.WIFSIGNALED(status):
        number = os.WTERMSIG(status)
        signal.signal(number, signal.SIG_DFL)
        os.kill(os.getpid(), number)
        time.sleep(1)
        raise SystemExit(128 + number)
    raise SystemExit(os.WEXITSTATUS(status))

main()
PY
}

run_local_vsock_namespace() {
  run_isolated_namespace net "$@"
}

run_node_isolated_namespace() {
  run_isolated_namespace node "$@"
}

case "$PLACEMENT_KIND" in
  host-pure)
    exec "$@"
    ;;
  guest-declared)
    # The test boots its own guest through the guest kit (`run_in_guest`); its
    # host-side half gets the local-mode vsock namespace.
    run_local_vsock_namespace "$@"
    ;;
  native-launcher)
    run_node_isolated_namespace "$@"
    ;;
  *)
    test_error placement_invalid 78
    ;;
esac
