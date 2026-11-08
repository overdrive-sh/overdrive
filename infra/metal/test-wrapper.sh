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
print("{}\t{}\t{}".format(
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
  IFS=$'\t' read -r PLACEMENT_KIND PLACEMENT_GROUP PLACEMENT_FIXTURE <<<"$PLACEMENT"
fi

run_local_vsock_namespace() {
  exec python3 - "$@" <<'PY'
import ctypes
import os
import sys

CLONE_NEWNET = 0x40000000
libc = ctypes.CDLL(None, use_errno=True)
libc.unshare.argtypes = [ctypes.c_int]
libc.unshare.restype = ctypes.c_int

def fail(step, error_number=None):
    value = "none" if error_number is None else str(error_number)
    print(
        f"OVERDRIVE_METAL_TEST_ERROR error=namespace_setup_failed step={step} errno={value}",
        file=sys.stderr,
    )
    raise SystemExit(70)

def unshare(step):
    if libc.unshare(CLONE_NEWNET) != 0:
        fail(step, ctypes.get_errno())

unshare("parent")
try:
    descriptor = os.open(
        "/proc/sys/net/vsock/child_ns_mode", os.O_WRONLY | os.O_CLOEXEC
    )
    try:
        os.write(descriptor, b"local\n")
    finally:
        os.close(descriptor)
except OSError as error:
    fail("child_mode", error.errno)

unshare("local")
try:
    descriptor = os.open("/proc/sys/net/vsock/ns_mode", os.O_RDONLY | os.O_CLOEXEC)
    try:
        mode = os.read(descriptor, 64).strip()
    finally:
        os.close(descriptor)
except OSError as error:
    fail("verify", error.errno)
if mode != b"local":
    fail("verify")

argv = sys.argv[1:]
if not argv:
    fail("verify")
os.execvpe(argv[0], argv, os.environ)
PY
}

if [ "$GUEST_SUITE_MODE" -eq 1 ]; then
  run_local_vsock_namespace "$@"
fi

case "$PLACEMENT_KIND" in
  host-pure)
    exec "$@"
    ;;
  native-launcher)
    run_local_vsock_namespace "$@"
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
        IN_GUEST_LANE_BINARY="${DISPATCHER_CANONICAL%/*}/in-guest/overdrive-init--overdrive-init"
        [ -f "$IN_GUEST_LANE_BINARY" ] && [ -x "$IN_GUEST_LANE_BINARY" ] || test_error lane_not_prepared 78
        ;;
    esac
    if [ "$PLACEMENT_KIND" = guest-group ]; then
      export OVERDRIVE_FWD_GUEST_GROUP="$PLACEMENT_GROUP"
    fi
    export OVERDRIVE_FWD_GUEST_EXACT="$TEST_NAME"
    run_local_vsock_namespace "$DISPATCHER_CANONICAL" --nextest-wrapper "$@"
    ;;
  *)
    test_error placement_invalid 78
    ;;
esac
