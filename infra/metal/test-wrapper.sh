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

GUEST_SUITE_MODE=0
if [ "${1:-}" = --guest-suite ]; then
  GUEST_SUITE_MODE=1
  shift
fi

# No Metal context means nextest has no reason to alter the configured runner.
if [ -z "${OVERDRIVE_METAL_RUN_ID:-}" ] && \
  [ -z "${OVERDRIVE_METAL_EXECUTION_CLASS:-}" ] && \
  [ -z "${OVERDRIVE_METAL_SOURCE_ROOT:-}" ] && \
  [ -z "${OVERDRIVE_METAL_TEST_DISPATCHER:-}" ]; then
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

if [ "$GUEST_SUITE_MODE" -eq 0 ]; then
  [ "$#" -gt 0 ] || test_error test_unclassified 78
  BINARY_ID="${NEXTEST_BINARY_ID:-}"
  TEST_NAME="${NEXTEST_TEST_NAME:-}"
  [ -n "$BINARY_ID" ] && [ -n "$TEST_NAME" ] || test_error test_unclassified 78
  PLACEMENT_FILE="$SOURCE_ROOT/infra/metal/test-placement.json"
  [ -f "$PLACEMENT_FILE" ] && [ ! -L "$PLACEMENT_FILE" ] || test_error placement_invalid 78

  set +e
  PLACEMENT="$(python3 - "$PLACEMENT_FILE" "$BINARY_ID" "$TEST_NAME" <<'PY'
import json
import re
import sys

class DuplicateKey(Exception):
    pass

def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKey(key)
        result[key] = value
    return result

def valid_descriptor(value):
    if not isinstance(value, dict) or not isinstance(value.get("kind"), str):
        return False
    kind = value["kind"]
    if kind in {"host-pure", "native-launcher"}:
        return set(value) == {"kind"}
    if kind == "guest-group":
        return set(value) == {"kind", "group"} and value.get("group") in {
            "probe", "core", "unload", "ulp", "mtls-relay", "cgroup-default",
            "stress", "nested-boundary",
        }
    if kind in {"guest-lane-operator", "guest-lane-owner"}:
        fixture = value.get("fixture")
        return (
            set(value) == {"kind", "fixture"}
            and isinstance(fixture, str)
            and re.fullmatch(r"integration::in_guest_lane::[A-Za-z0-9_]+", fixture) is not None
        )
    return False

try:
    with open(sys.argv[1], encoding="utf-8") as source:
        policy = json.load(source, object_pairs_hook=unique_object)
except DuplicateKey:
    raise SystemExit(11)
except Exception:
    raise SystemExit(13)

if (
    not isinstance(policy, dict)
    or set(policy) != {"format", "placements"}
    or policy.get("format") != "overdrive-metal-test-placement-v1"
    or not isinstance(policy.get("placements"), dict)
):
    raise SystemExit(13)

for binary_id, tests in policy["placements"].items():
    if not isinstance(binary_id, str) or not binary_id or not isinstance(tests, dict):
        raise SystemExit(13)
    for test_name, descriptor in tests.items():
        if not isinstance(test_name, str) or not test_name or not valid_descriptor(descriptor):
            raise SystemExit(13)

descriptor = policy["placements"].get(sys.argv[2], {}).get(sys.argv[3])
if descriptor is None:
    raise SystemExit(12)
print("{}\x1f{}\x1f{}".format(
    descriptor["kind"],
    descriptor.get("group", ""),
    descriptor.get("fixture", ""),
))
PY
)"
  PARSER_STATUS=$?
  set -e
  case "$PARSER_STATUS" in
    0) ;;
    11) test_error placement_ambiguous 78 ;;
    12) test_error test_unclassified 78 ;;
    *) test_error placement_invalid 78 ;;
  esac
  IFS=$'\x1f' read -r PLACEMENT_KIND PLACEMENT_GROUP PLACEMENT_FIXTURE <<<"$PLACEMENT"
fi

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

if [ "$GUEST_SUITE_MODE" -eq 1 ]; then
  run_local_vsock_namespace "$@"
fi

case "$PLACEMENT_KIND" in
  host-pure)
    exec "$@"
    ;;
  native-launcher)
    run_node_isolated_namespace "$@"
    ;;
  guest-group|guest-lane-operator|guest-lane-owner)
    DISPATCHER="${OVERDRIVE_METAL_TEST_DISPATCHER:-}"
    [ -n "$DISPATCHER" ] || test_error lane_not_prepared 78
    DISPATCHER_CANONICAL="$(realpath -e -- "$DISPATCHER" 2>/dev/null)" || test_error lane_not_prepared 78
    case "$DISPATCHER_CANONICAL" in
      "$SOURCE_ROOT"/target/*) ;;
      *) test_error lane_not_prepared 78 ;;
    esac
    [ -f "$DISPATCHER_CANONICAL" ] && [ -x "$DISPATCHER_CANONICAL" ] || test_error lane_not_prepared 78
    case "$PLACEMENT_KIND" in
      guest-lane-operator|guest-lane-owner)
        IN_GUEST_LANE_BINARY="$SOURCE_ROOT/target/guest-kit/in-guest/overdrive-init--overdrive-init"
        [ -f "$IN_GUEST_LANE_BINARY" ] && [ -x "$IN_GUEST_LANE_BINARY" ] || test_error lane_not_prepared 78
        ;;
    esac
    if [ "$PLACEMENT_KIND" = guest-group ]; then
      export OVERDRIVE_FWD_GUEST_GROUP="$PLACEMENT_GROUP"
    elif [ "$PLACEMENT_KIND" = guest-lane-operator ] || [ "$PLACEMENT_KIND" = guest-lane-owner ]; then
      export OVERDRIVE_FWD_GUEST_LANE_KIND="$PLACEMENT_KIND"
      export OVERDRIVE_FWD_GUEST_LANE_FIXTURE="$PLACEMENT_FIXTURE"
      export OVERDRIVE_FWD_GUEST_NEXTTEST_ELF="$1"
      GUEST_ARGUMENTS="$(python3 - "$@" <<'PY'
import sys

arguments = sys.argv[2:]
print(f"{len(arguments)}:" + ";".join(argument.encode().hex() for argument in arguments))
PY
)" || test_error test_unclassified 78
      export OVERDRIVE_FWD_GUEST_NEXTTEST_ARGV="$GUEST_ARGUMENTS"
    fi
    export OVERDRIVE_FWD_GUEST_EXACT="$TEST_NAME"
    run_local_vsock_namespace "$DISPATCHER_CANONICAL" --nextest-wrapper "$@"
    ;;
  *)
    test_error placement_invalid 78
    ;;
esac
