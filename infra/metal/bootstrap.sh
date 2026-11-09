#!/usr/bin/env bash
# Bootstrap the bare-metal probe box FROM YOUR LAPTOP.
#
# rsyncs the working tree up and runs provisioning over ssh. rsync rather than
# `git clone` on purpose: all the spike probe code lives in spike-scratch/,
# which is gitignored (.claude/rules/spike.md), so a fresh clone would arrive
# with none of it.
#
# Idempotent: re-run freely. Subsequent syncs are incremental.
#
# Locking model (two lock levels, always taken global -> checkout):
#   * GLOBAL lease (/run/lock/overdrive-metal-shared.lock): one holder per box.
#     --run, --shell and bootstrap (provisioning) take it, because they mutate
#     node-global kernel state (nft tables, routes, cgroups, netns, KVM).
#   * CHECKOUT lock (/run/lock/overdrive-metal-tree-<sha256>.lock): one holder
#     per canonical remote checkout directory. --sync-only and --pull take only
#     this lock; --run also takes it after the global lease. The durable command
#     retains it for the whole command, excluding sync and pull after controller
#     disconnection. See the run-mode comments below.
#
# Usage:
#   infra/metal/bootstrap.sh root@1.2.3.4
#   infra/metal/bootstrap.sh root@1.2.3.4 --data-disk /dev/sdb
#   infra/metal/bootstrap.sh root@1.2.3.4 --sync-only
#   infra/metal/bootstrap.sh root@1.2.3.4 --run -- cargo nextest run
#   infra/metal/bootstrap.sh root@1.2.3.4 --collect <RUN_ID>
#   infra/metal/bootstrap.sh root@1.2.3.4 --pull -- specs/quint/<subsystem>/evidence
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)

checkout_identity_invalid() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=checkout_identity_invalid\n' >&2
  exit 64
}

remote_subdir_invalid() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=remote_subdir_invalid\n' >&2
  exit 64
}

checkout_identity_conflict() {
  if [ "${RUN_MODE:-0}" -eq 1 ] && [ "${RUN_CONTROL_OPEN:-0}" -eq 1 ]; then
    abort_registered_run checkout_identity_conflict
  fi
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=checkout_identity_conflict\n' >&2
  exit 78
}

source_mismatch() {
  if [ "${RUN_MODE:-0}" -eq 1 ] && [ "${RUN_CONTROL_OPEN:-0}" -eq 1 ]; then
    abort_registered_run source_mismatch
  fi
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=source_mismatch\n' >&2
  exit 78
}

rsync_prerequisite_failed() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=runner_preparation_failed prerequisite=rsync\n' >&2
  if [ "${RUN_MODE:-0}" -eq 1 ] && [ "${RUN_CONTROL_OPEN:-0}" -eq 1 ]; then
    abort_registered_run runner_preparation_failed
  fi
  exit 75
}

run_preparation_failed() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=runner_preparation_failed\n' >&2
  if [ "${RUN_MODE:-0}" -eq 1 ] && [ "${RUN_CONTROL_OPEN:-0}" -eq 1 ]; then
    abort_registered_run runner_preparation_failed
  fi
  exit 75
}

invalid_mode() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=invalid_mode\n' >&2
  exit 64
}

unsupported_ordinary_command() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=unsupported_ordinary_command\n' >&2
  exit 64
}

execution_override_refused() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=execution_override_refused\n' >&2
  exit 64
}

artifact_scope_mismatch() {
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=artifact_scope_mismatch\n' >&2
  exit 64
}

validate_ordinary_command() {
  local index=0
  local key value argument subcommand package="" binary=""
  local has_doc=0
  local separator=0
  local position
  local -a seen_environment=()
  ORDINARY_NEXTTEST=0
  ORDINARY_SUITE_RUN=0
  if [ "${#RUN_ARGS[@]}" -eq 0 ]; then
    unsupported_ordinary_command
  fi
  if [ "${RUN_ARGS[0]}" = env ] || [ "${RUN_ARGS[0]}" = /usr/bin/env ]; then
    index=1
    while [ "$index" -lt "${#RUN_ARGS[@]}" ] && [[ "${RUN_ARGS[$index]}" == *=* ]]; do
      argument="${RUN_ARGS[$index]}"
      key="${argument%%=*}"
      value="${argument#*=}"
      [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || unsupported_ordinary_command
      for existing in "${seen_environment[@]}"; do
        [ "$existing" != "$key" ] || unsupported_ordinary_command
      done
      seen_environment+=("$key")
      case "$key" in
        OVERDRIVE_BPF_NATIVE)
          [ "$value" = 1 ] || unsupported_ordinary_command
          ;;
        OVERDRIVE_FWD_GUEST_GROUP)
          case "$value" in
            all|probe|core|unload|ulp|mtls-relay|cgroup-default|stress|nested-boundary) ;;
            *) unsupported_ordinary_command ;;
          esac
          ;;
        OVERDRIVE_FWD_GUEST_EXACT|RUST_BACKTRACE|RUST_LOG|NEXTEST_PROFILE|\
        PROPTEST_CASES|PROPTEST_DISABLE_FAILURE_PERSISTENCE|PROPTEST_FORK|\
        PROPTEST_MAX_DEFAULT_SIZE_RANGE|PROPTEST_MAX_FLAT_MAP_REGENS|\
        PROPTEST_MAX_GLOBAL_REJECTS|PROPTEST_MAX_LOCAL_REJECTS|\
        PROPTEST_MAX_SHRINK_ITERS|PROPTEST_MAX_SHRINK_TIME|\
        PROPTEST_RNG_ALGORITHM|PROPTEST_RNG_SEED|PROPTEST_TIMEOUT|PROPTEST_VERBOSE)
          if ! printf '%s' "$value" | iconv -f UTF-8 -t UTF-8 >/dev/null 2>&1; then
            unsupported_ordinary_command
          fi
          ;;
        *) unsupported_ordinary_command ;;
      esac
      index=$((index + 1))
    done
  fi
  [ "$index" -lt "${#RUN_ARGS[@]}" ] || unsupported_ordinary_command
  [ "${RUN_ARGS[$index]}" = cargo ] || unsupported_ordinary_command
  COMMAND_INDEX="$index"
  index=$((index + 1))
  [ "$index" -lt "${#RUN_ARGS[@]}" ] || unsupported_ordinary_command
  subcommand="${RUN_ARGS[$index]}"
  case "$subcommand" in
    build|check|clippy|fmt) ;;
    test)
      position=$((index + 1))
      while [ "$position" -lt "${#RUN_ARGS[@]}" ]; do
        argument="${RUN_ARGS[$position]}"
        [ "$argument" != -- ] || break
        [ "$argument" != --doc ] || has_doc=1
        position=$((position + 1))
      done
      [ "$has_doc" -eq 1 ] || unsupported_ordinary_command
      ;;
    nextest)
      [ "$((index + 1))" -lt "${#RUN_ARGS[@]}" ] && \
        [ "${RUN_ARGS[$((index + 1))]}" = run ] || unsupported_ordinary_command
      ORDINARY_NEXTTEST=1
      ;;
    xtask)
      [ "$((index + 1))" -lt "${#RUN_ARGS[@]}" ] || unsupported_ordinary_command
      case "${RUN_ARGS[$((index + 1))]}" in
        guest-kit-prepare|bpf-build|bpf-clippy|bpf-unit|kmod-build|\
        dst-lint|cloexec-lint|conformance-lint|yaml-free-cli|quint|mutants) ;;
        *) unsupported_ordinary_command ;;
      esac
      ;;
    run)
      position=$((index + 1))
      while [ "$position" -lt "${#RUN_ARGS[@]}" ]; do
        argument="${RUN_ARGS[$position]}"
        [ "$argument" != -- ] || { separator=1; break; }
        case "$argument" in
          -p|--package)
            position=$((position + 1))
            [ "$position" -lt "${#RUN_ARGS[@]}" ] || unsupported_ordinary_command
            [ -z "$package" ] || unsupported_ordinary_command
            package="${RUN_ARGS[$position]}"
            ;;
          --package=*)
            [ -z "$package" ] || unsupported_ordinary_command
            package="${argument#--package=}"
            ;;
          --bin)
            position=$((position + 1))
            [ "$position" -lt "${#RUN_ARGS[@]}" ] || unsupported_ordinary_command
            [ -z "$binary" ] || unsupported_ordinary_command
            binary="${RUN_ARGS[$position]}"
            ;;
          --bin=*)
            [ -z "$binary" ] || unsupported_ordinary_command
            binary="${argument#--bin=}"
            ;;
        esac
        position=$((position + 1))
      done
      case "$package/$binary" in
        overdrive-testing/overdrive-fwd-guest-suite)
          ORDINARY_SUITE_RUN=1
          ;;
        overdrive-sim/dst|overdrive-control-plane/openapi) ;;
        *) unsupported_ordinary_command ;;
      esac
      ;;
    *) unsupported_ordinary_command ;;
  esac

  position=$((COMMAND_INDEX + 1))
  while [ "$position" -lt "${#RUN_ARGS[@]}" ]; do
    argument="${RUN_ARGS[$position]}"
    [ "$argument" != -- ] || break
    case "$argument" in
      --config|--config=*|--config-file|--config-file=*|\
      --tool-config-file|--tool-config-file=*|--user-config-file|\
      --user-config-file=*|--debugger|--debugger=*|--tracer|--tracer=*)
        execution_override_refused
        ;;
      --archive-file|--archive-file=*|--archive-format|--archive-format=*|\
      --extract-to|--extract-to=*|--extract-overwrite|\
      --persist-extract-tempdir|--cargo-metadata|--cargo-metadata=*|\
      --binaries-metadata|--binaries-metadata=*|--workspace-remap|\
      --workspace-remap=*|--target-dir-remap|--target-dir-remap=*|\
      --build-dir-remap|--build-dir-remap=*)
        artifact_scope_mismatch
        ;;
    esac
    position=$((position + 1))
  done
}

validate_remote_artifact_paths() {
  local index argument value candidate remote_command result
  local -a artifact_paths=()
  index=$((COMMAND_INDEX + 1))
  while [ "$index" -lt "${#RUN_ARGS[@]}" ]; do
    argument="${RUN_ARGS[$index]}"
    [ "$argument" != -- ] || break
    case "$argument" in
      --manifest-path=*) artifact_paths+=("${argument#--manifest-path=}") ;;
      --target-dir=*) artifact_paths+=("${argument#--target-dir=}") ;;
      --manifest-path|--target-dir)
        index=$((index + 1))
        [ "$index" -lt "${#RUN_ARGS[@]}" ] || artifact_scope_mismatch
        artifact_paths+=("${RUN_ARGS[$index]}")
        ;;
    esac
    index=$((index + 1))
  done
  for value in "${artifact_paths[@]}"; do
    if [[ "$value" = /* ]]; then
      candidate="$value"
    else
      candidate="${REMOTE_DIR}/${value}"
    fi
    printf -v remote_command \
      'candidate=%q; checkout=%q; scratch=%q; canonical=$(realpath -m -- "$candidate") || exit 1; case "$canonical" in "$checkout"|"$checkout"/*|"$scratch"|"$scratch"/*) printf "ok\\n" ;; *) printf "invalid\\n" ;; esac' \
      "$candidate" "$REMOTE_DIR" "$METAL_RUN_TMP"
    result="$(ssh "${SSH_OPTS[@]}" "${TARGET}" "$remote_command")" || artifact_scope_mismatch
    [ "$result" = ok ] || artifact_scope_mismatch
  done
}

[ $# -ge 1 ] || { echo "usage: $0 <user@host> [--data-disk DEV] [--sync-only | --pull -- PATH... | --run [--exclusive] -- CMD... | --shell | --collect ID]" >&2; exit 1; }
TARGET="$1"; shift

DATA_DISK=""
BREAK_RAID_DISK=""
SYNC_ONLY=0
PULL_MODE=0
RUN_MODE=0
EXCLUSIVE_MODE=0
SHELL_MODE=0
COLLECT_MODE=0
COLLECT_RUN_ID=""
NO_SYNC=0
NO_SUDO=0
RUN_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --data-disk) DATA_DISK="$2"; shift 2 ;;
    --break-raid-disk) BREAK_RAID_DISK="$2"; shift 2 ;;
    --sync-only) SYNC_ONLY=1; shift ;;
    --pull) PULL_MODE=1; shift ;;
    --run) RUN_MODE=1; shift ;;
    --exclusive) EXCLUSIVE_MODE=1; shift ;;
    --shell) SHELL_MODE=1; shift ;;
    --collect)
      [ $# -ge 2 ] || { echo "FATAL: --collect requires a run id" >&2; exit 64; }
      COLLECT_MODE=1; COLLECT_RUN_ID="$2"; shift 2 ;;
    --no-sync) NO_SYNC=1; shift ;;
    --no-sudo) NO_SUDO=1; shift ;;
    --) shift; RUN_ARGS=("$@"); break ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [ "$EXCLUSIVE_MODE" -eq 1 ] && [ "$RUN_MODE" -eq 0 ]; then
  invalid_mode
fi
if [ "$EXCLUSIVE_MODE" -eq 1 ] && \
  { [ "$SYNC_ONLY" -eq 1 ] || [ "$PULL_MODE" -eq 1 ] || \
    [ "$SHELL_MODE" -eq 1 ] || [ "$COLLECT_MODE" -eq 1 ]; }; then
  invalid_mode
fi

[ $((SYNC_ONLY + PULL_MODE + RUN_MODE + SHELL_MODE + COLLECT_MODE)) -le 1 ] || {
  echo "FATAL: choose only one of --sync-only, --pull, --run, --shell, or --collect" >&2
  exit 1
}
[ "${RUN_MODE}" -eq 0 ] || [ "${#RUN_ARGS[@]}" -gt 0 ] || {
  echo "FATAL: --run requires a command after --" >&2
  exit 1
}
[ "${PULL_MODE}" -eq 0 ] || [ "${#RUN_ARGS[@]}" -gt 0 ] || {
  echo "FATAL: --pull requires one or more repo-relative paths after --" >&2
  exit 1
}
if [ "$RUN_MODE" -eq 1 ] && [ "$EXCLUSIVE_MODE" -eq 0 ]; then
  validate_ordinary_command
  if [ "$ORDINARY_NEXTTEST" -eq 1 ]; then
    [ -f "${REPO}/.config/nextest.toml" ] || execution_override_refused
    grep -Fq 'command = { command-line = "infra/metal/test-wrapper.sh", relative-to = "workspace-root" }' \
      "${REPO}/.config/nextest.toml" || execution_override_refused
    grep -Fq 'target-runner = "within-wrapper"' "${REPO}/.config/nextest.toml" \
      || execution_override_refused
    grep -Fq 'run-wrapper = "metal"' "${REPO}/.config/nextest.toml" \
      || execution_override_refused
    for config in "${REPO}/.cargo/config.toml" "${REPO}/.cargo/config"; do
      if [ -f "$config" ] && \
        awk '!/^[[:space:]]*#/ && /^[[:space:]]*runner[[:space:]]*=/' \
          "$config" | grep -q .; then
        execution_override_refused
      fi
    done
  fi
fi
# Pulled paths are validated here as well as by `cargo xtask metal pull`, so a
# direct caller gets the same refusal: repo-relative, no `.`/`..`/`.git`/empty
# component, and a charset that needs no shell quoting on either side.
if [ "${PULL_MODE}" -eq 1 ]; then
  for _path in "${RUN_ARGS[@]}"; do
    _norm="${_path%/}"
    [[ "${_norm}" =~ ^[A-Za-z0-9._/-]+$ ]] \
      && [[ "${_norm}" != /* ]] \
      && [[ "/${_norm}/" != *//* ]] \
      && [[ "/${_norm}/" != */./* ]] \
      && [[ "/${_norm}/" != */../* ]] \
      && [[ "/${_norm}/" != */.git/* ]] \
      && [[ "${_norm}/" != target/* ]] || {
      echo "FATAL: --pull path '${_path}' must be repo-relative, with no '.', '..', '.git' or empty component, not under target/, and only [A-Za-z0-9._/-]" >&2
      exit 64
    }
  done
fi

if [ "${COLLECT_MODE}" -eq 1 ] && ! [[ "${COLLECT_RUN_ID}" =~ ^[0-9a-f]{24}$ ]]; then
  echo "OVERDRIVE_METAL_RUNNER_ERROR error=invalid_run_id" >&2
  exit 64
fi

# Collection is keyed only by the durable run id. Every source operation uses
# a local checkout identity before contacting the remote host.
CHECKOUT_KIND=""
CHECKOUT_KEY=""
CHECKOUT_IDENTITY_B64=""
CHECKOUT_BINDING_B64=""
REMOTE_SUBDIR=""
REMOTE_SUBDIR_IS_OVERRIDE=0
if [ "${COLLECT_MODE}" -eq 0 ]; then
  GIT_BARE="$(git -C "${REPO}" rev-parse --is-bare-repository 2>/dev/null)" \
    || checkout_identity_invalid
  [ "${GIT_BARE}" = false ] || checkout_identity_invalid

  GIT_TOP="$(git -C "${REPO}" rev-parse --show-toplevel 2>/dev/null)" \
    || checkout_identity_invalid
  REPO="$(cd "${GIT_TOP}" 2>/dev/null && pwd -P)" || checkout_identity_invalid
  GIT_DIR="$(git -C "${REPO}" rev-parse --absolute-git-dir 2>/dev/null)" \
    || checkout_identity_invalid
  GIT_COMMON="$(git -C "${REPO}" rev-parse --git-common-dir 2>/dev/null)" \
    || checkout_identity_invalid
  case "${GIT_COMMON}" in
    /*) ;;
    *) GIT_COMMON="${REPO}/${GIT_COMMON}" ;;
  esac
  GIT_DIR="$(cd "${GIT_DIR}" 2>/dev/null && pwd -P)" || checkout_identity_invalid
  GIT_COMMON="$(cd "${GIT_COMMON}" 2>/dev/null && pwd -P)" || checkout_identity_invalid

  if [ "${GIT_DIR}" = "${GIT_COMMON}" ]; then
    GIT_REF="$(git -C "${REPO}" symbolic-ref --quiet HEAD 2>/dev/null)" \
      || checkout_identity_invalid
    case "${GIT_REF}" in
      refs/heads/?*) ;;
      *) checkout_identity_invalid ;;
    esac
    git -C "${REPO}" check-ref-format "${GIT_REF}" >/dev/null 2>&1 \
      || checkout_identity_invalid
    CHECKOUT_KIND="branch"
    CHECKOUT_IDENTITY_B64="$(printf '%s\0%s' "${REPO}" "${GIT_REF}" | base64 | tr -d '\r\n')"
    CHECKOUT_KEY="$({
      printf 'overdrive-metal-checkout-v1\0branch\0'
      printf '%s\0%s' "${REPO}" "${GIT_REF}"
    } | shasum -a 256 | awk '{print $1}')"
    CHECKOUT_DEFAULT_PREFIX="overdrive-branches"
  else
    CHECKOUT_KIND="worktree"
    CHECKOUT_IDENTITY_B64="$(printf '%s' "${GIT_DIR}" | base64 | tr -d '\r\n')"
    CHECKOUT_KEY="$(printf 'overdrive-metal-checkout-v1\0worktree\0%s' "${GIT_DIR}" \
      | shasum -a 256 | awk '{print $1}')"
    CHECKOUT_DEFAULT_PREFIX="overdrive-worktrees"
  fi

  if [ -n "${OVERDRIVE_METAL_REMOTE_SUBDIR:-}" ]; then
    REMOTE_SUBDIR="${OVERDRIVE_METAL_REMOTE_SUBDIR}"
    [[ "${REMOTE_SUBDIR}" =~ ^[A-Za-z0-9._/-]+$ ]] || remote_subdir_invalid
    [[ "${REMOTE_SUBDIR}" != /* ]] || remote_subdir_invalid
    case "/${REMOTE_SUBDIR}/" in
      *//*|*/./*|*/../*) remote_subdir_invalid ;;
    esac
    REMOTE_SUBDIR_IS_OVERRIDE=1
  else
    REMOTE_SUBDIR="${CHECKOUT_DEFAULT_PREFIX}/${CHECKOUT_KEY}"
  fi

  CHECKOUT_BINDING_B64="$(printf \
    'format=overdrive-metal-checkout-v1\nkind=%s\ncheckout_key=%s\nidentity_b64=%s\n' \
    "${CHECKOUT_KIND}" "${CHECKOUT_KEY}" "${CHECKOUT_IDENTITY_B64}" \
    | base64 | tr -d '\r\n')"
fi

RUN_ID=""
EXECUTION_CLASS="ordinary"
if [ "${RUN_MODE}" -eq 1 ]; then
  RUN_ID="$(printf '%s-%s-%s' "$$" "$(date +%s%N)" "${RANDOM}" | shasum -a 256 | cut -c1-24)"
  [ "$EXCLUSIVE_MODE" -eq 0 ] || EXECUTION_CLASS="exclusive"
  printf 'OVERDRIVE_METAL_RUN_ID=%s\n' "${RUN_ID}" >&2
fi

METAL_LOCK_PATH="${OVERDRIVE_METAL_LOCK_PATH:-/run/lock/overdrive-metal-shared.lock}"
METAL_OWNER_PATH="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"
METAL_LEASE_TIMEOUT_SECONDS="${OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS:-120}"
METAL_RUNS_ROOT="/var/lib/overdrive-metal/runs"
LEASE_TOKEN="${RUN_ID:-$(printf '%s-%s-%s' "$$" "$(date +%s)" "${RANDOM}" | shasum -a 256 | cut -c1-24)}"
LEASE_TMP=""
LEASE_PID=""
RUN_CONTROL_OPEN=0
START_SENT=0
# Second (per-tree) holder, used by --run only; its release stream is fd 6.
TREE_PID=""

# Release the per-tree holder: closing its release stream (fd 6) ends the
# remote holder, which drops the flock and its owner metadata.
release_tree_lease() {
  exec 6>&- 2>/dev/null || true
  if [ -n "${TREE_PID}" ]; then
    wait "${TREE_PID}" 2>/dev/null || true
    TREE_PID=""
  fi
}

cleanup_lease() {
  if [ -n "${LEASE_TMP}" ]; then
    release_tree_lease
    if [ "${RUN_MODE}" -eq 1 ] && [ "${START_SENT}" -eq 0 ] && [ "${RUN_CONTROL_OPEN}" -eq 1 ]; then
      printf 'ABORT\n' >&9 2>/dev/null || true
    fi
    exec 9>&- 2>/dev/null || true
    RUN_CONTROL_OPEN=0
    if [ -n "${LEASE_PID}" ]; then
      wait "${LEASE_PID}" 2>/dev/null || true
    fi
    rm -rf "${LEASE_TMP}"
  fi
}
trap cleanup_lease EXIT
trap 'exit 130' INT TERM HUP

log() { printf '\n########## [bootstrap] %s\n' "$*"; }

ensure_checkout_binding() {
  local mode="$1"
  local probe_script probe_command result writer_script writer_command

  probe_script='set -eu
dir=$1
expected_b64=$2
mode=$3
if [ -L "$dir" ]; then printf "conflict\n"; exit 0; fi
if [ ! -e "$dir" ]; then
  if [ "$mode" = bind ]; then printf "new\n"; else printf "conflict\n"; fi
  exit 0
fi
if [ ! -d "$dir" ]; then printf "conflict\n"; exit 0; fi
binding="$dir/.overdrive-metal-checkout.v1"
if [ -L "$binding" ]; then printf "conflict\n"; exit 0; fi
if [ -e "$binding" ]; then
  if [ ! -f "$binding" ]; then printf "conflict\n"; exit 0; fi
  if printf "%s" "$expected_b64" | base64 -d | cmp -s "$binding" -; then
    printf "valid\n"
  else
    printf "conflict\n"
  fi
  exit 0
fi
shopt -s dotglob nullglob
entries=("$dir"/*)
if [ "${#entries[@]}" -ne 0 ]; then
  printf "conflict\n"
elif [ "$mode" = bind ]; then
  printf "new\n"
else
  printf "conflict\n"
fi'
  printf -v probe_command 'bash -c %q metal-checkout-binding %q %q %q' \
    "${probe_script}" "${REMOTE_DIR}" "${CHECKOUT_BINDING_B64}" "${mode}"
  result="$(ssh "${SSH_OPTS[@]}" "${TARGET}" "${probe_command}")" || return "$?"

  case "${result}" in
    valid) return 0 ;;
    new)
      [ "${mode}" = bind ] || {
        if [ "${NO_SYNC}" -eq 1 ]; then source_mismatch; fi
        checkout_identity_conflict
      }
      writer_script='set -eu
dir=$1
expected_b64=$2
binding="$dir/.overdrive-metal-checkout.v1"
mkdir -p "$dir"
if [ -e "$binding" ] || [ -L "$binding" ]; then exit 78; fi
tmp="$binding.tmp.$$.${RANDOM:-0}"
umask 077
printf "%s" "$expected_b64" | base64 -d > "$tmp"
mv -f -- "$tmp" "$binding"'
      printf -v writer_command 'bash -c %q metal-checkout-binding-writer %q %q' \
        "${writer_script}" "${REMOTE_DIR}" "${CHECKOUT_BINDING_B64}"
      ssh "${SSH_OPTS[@]}" "${TARGET}" "${writer_command}"
      return 0
      ;;
    conflict)
      if [ "${NO_SYNC}" -eq 1 ]; then source_mismatch; fi
      checkout_identity_conflict
      ;;
    *)
      if [ "${NO_SYNC}" -eq 1 ]; then source_mismatch; fi
      checkout_identity_conflict
      ;;
  esac
}

# ---------------------------------------------------------------------------
# rsync flavour — prefer a real rsync 3.x, tolerate openrsync.
#
# macOS 15+ ships **openrsync**, which advertises "protocol version 29 / rsync
# 2.6.9 compatible". It handles -a/-z/--delete/--exclude/-e correctly, so this
# script works with it — but it is the degraded path, and not only because it
# lacks `--info=` (that is rsync 3.1+):
#
#   protocol 29 has no INCREMENTAL RECURSION. It builds the complete file list
#   before transferring anything, so every re-sync walks and buffers the whole
#   tree. rsync 3.x streams it. On a tree you re-sync after every probe edit,
#   that difference compounds all day.
#
# So: `brew install rsync` (3.4.4 as of 2026-08) and this picks it up
# automatically. Override with RSYNC_BIN=/path/to/rsync.
# ---------------------------------------------------------------------------
if [ -z "${RSYNC_BIN:-}" ]; then
  RSYNC_BIN="rsync"
  for cand in /opt/homebrew/bin/rsync /usr/local/bin/rsync; do
    [ -x "${cand}" ] && { RSYNC_BIN="${cand}"; break; }
  done
fi

STATS_FLAG=(--stats)
if "${RSYNC_BIN}" --info=stats1 --version >/dev/null 2>&1; then
  STATS_FLAG=(--info=stats1)
else
  log "using $("${RSYNC_BIN}" --version 2>&1 | head -1)"
  echo "     ^ openrsync (protocol 29): no incremental recursion, so re-syncs" >&2
  echo "       walk the whole tree. Faster: brew install rsync" >&2
fi

# ---------------------------------------------------------------------------
log "preflight: ssh to ${TARGET}"
# ---------------------------------------------------------------------------
# Fail here with a clear message rather than halfway through an rsync.
if ! ssh "${SSH_OPTS[@]}" -o ConnectTimeout=10 -o BatchMode=yes \
        "${TARGET}" 'echo ok' >/dev/null 2>&1; then
  echo "FATAL: cannot ssh to ${TARGET} non-interactively." >&2
  echo >&2
  echo "  Most likely, in order:" >&2
  echo "  1. THE SERVER IS STILL INSTALLING. Scaleway's installer runs its own" >&2
  echo "     sshd, so the port answers but your key is not in the target OS yet." >&2
  echo "     Symptom is 'Permission denied (publickey)' or 'Connection closed'." >&2
  echo "     Check the console and wait — bare-metal installs take 10-30 min." >&2
  echo "  2. WRONG USERNAME. Scaleway's Ubuntu images log in as 'ubuntu', NOT" >&2
  echo "     'root' — root's authorized_keys carries a forced-command banner" >&2
  echo "     that says exactly that. Try: ssh ubuntu@${TARGET#*@}" >&2
  echo "  3. The key is passphrase-protected and not loaded: ssh-add ~/.ssh/id_rsa" >&2
  echo "     (this preflight uses BatchMode, which cannot prompt)." >&2
  echo >&2
  echo "  Diagnose: ssh -vv ${SSH_OPTS[*]} ${TARGET}" >&2
  echo "  Look for 'Will attempt key' (your key was found) vs 'no such identity'." >&2
  exit 1
fi
# Resolve the remote home and selected directory without creating anything.
# The canonical absolute directory also names the checkout-exclusion lock.
REMOTE_HOME_RAW="$(ssh "${SSH_OPTS[@]}" "${TARGET}" 'echo $HOME')"
REMOTE_HOME="${REMOTE_HOME_RAW}"
REMOTE_DIR=""
if [ "${COLLECT_MODE}" -eq 0 ]; then
  REMOTE_PATH_RESOLVER='set -eu
home=$(realpath -e -- "$1") || { printf "invalid\n"; exit 0; }
subdir=$2
check_symlinks=$3
candidate="$home/$subdir"
if [ "$check_symlinks" = 1 ]; then
  remaining=$subdir
  path=$home
  while [ -n "$remaining" ]; do
    component=${remaining%%/*}
    path="$path/$component"
    [ ! -L "$path" ] || { printf "invalid\n"; exit 0; }
    if [ "$remaining" = "$component" ]; then break; fi
    remaining=${remaining#*/}
  done
fi
canonical=$(realpath -m -- "$candidate") || { printf "invalid\n"; exit 0; }
case "$canonical" in
  "$home"/*) printf "ok\n%s\n%s\n" "$home" "$canonical" ;;
  *) printf "invalid\n" ;;
esac'
  printf -v REMOTE_PATH_COMMAND 'bash -c %q metal-remote-path %q %q %q' \
    "${REMOTE_PATH_RESOLVER}" "${REMOTE_HOME_RAW}" "${REMOTE_SUBDIR}" \
    "${REMOTE_SUBDIR_IS_OVERRIDE}"
  REMOTE_PATH_RESULT="$(ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_PATH_COMMAND}")"
  [ "${REMOTE_PATH_RESULT}" != invalid ] || remote_subdir_invalid
  [[ "${REMOTE_PATH_RESULT}" == $'ok\n'* ]] || remote_subdir_invalid
  REMOTE_PATH_RESULT="${REMOTE_PATH_RESULT#*$'\n'}"
  [[ "${REMOTE_PATH_RESULT}" == *$'\n'* ]] || remote_subdir_invalid
  REMOTE_HOME="${REMOTE_PATH_RESULT%%$'\n'*}"
  REMOTE_DIR="${REMOTE_PATH_RESULT#*$'\n'}"
fi

# The login user and home vary by image: root on some, `ubuntu` on Scaleway's.
REMOTE_USER="$(ssh "${SSH_OPTS[@]}" "${TARGET}" 'id -un')"
if [ "${REMOTE_USER}" = "root" ]; then
  SUDO=""
else
  SUDO="sudo -n"
  if ! ssh "${SSH_OPTS[@]}" "${TARGET}" 'sudo -n true' 2>/dev/null; then
    echo "FATAL: ${REMOTE_USER}@ has no passwordless sudo — provisioning needs root." >&2
    echo "  Either log in as root, or grant NOPASSWD sudo to ${REMOTE_USER}." >&2
    exit 1
  fi
fi
log "remote user=${REMOTE_USER} home=${REMOTE_HOME} sudo='${SUDO:-<none, already root>}'"

if [ "${COLLECT_MODE}" -eq 0 ]; then
  TREE_HASH="$(printf '%s' "${REMOTE_DIR}" | shasum -a 256 | awk '{print $1}')"
  METAL_TREE_LOCK_PATH="${OVERDRIVE_METAL_TREE_LOCK_PATH:-/run/lock/overdrive-metal-tree-${TREE_HASH}.lock}"
  METAL_TREE_OWNER_PATH="${OVERDRIVE_METAL_TREE_OWNER_PATH:-/run/lock/overdrive-metal-tree-${TREE_HASH}.owner}"
fi
METAL_RUN_TMP=""
METAL_TEST_DISPATCHER=""
if [ "${RUN_MODE}" -eq 1 ]; then
  METAL_RUN_TMP="${REMOTE_DIR}/target/metal-run-tmp/${RUN_ID}"
  METAL_TEST_DISPATCHER="${REMOTE_DIR}/target/debug/overdrive-fwd-guest-suite"
  if [ "${EXCLUSIVE_MODE}" -eq 0 ]; then
    validate_remote_artifact_paths
  fi
fi

collect_remote() {
  local helper_b64 remote_command status
  helper_b64="$(base64 < "${REPO}/infra/metal/lease-holder.sh" | tr -d '\n')"
  # Decode the repository-local collector into bash -c's command argument.
  # The collector itself validates the run id again before deriving any unit
  # or path, and it never opens the canonical lease.
  printf -v remote_command \
    'lease_script=$(printf %%s %q | base64 -d); exec %s bash -c "$lease_script" overdrive-metal-lease --collect %q %q %q %q %q' \
    "${helper_b64}" "${SUDO}" "${COLLECT_RUN_ID:-$1}" "${METAL_RUNS_ROOT}" \
    "${METAL_LEASE_TIMEOUT_SECONDS}" "${METAL_LOCK_PATH}" "${METAL_OWNER_PATH}"
  set +e
  ssh "${SSH_OPTS[@]}" "${TARGET}" "${remote_command}"
  status=$?
  set -e
  return "${status}"
}

abort_registered_run() {
  local error="$1"
  local status
  if [ "${RUN_CONTROL_OPEN}" -eq 1 ]; then
    printf 'ABORT error=%s\n' "$error" >&9 2>/dev/null || true
    exec 9>&- || true
    RUN_CONTROL_OPEN=0
  fi
  if [ -n "${LEASE_PID}" ]; then
    wait "${LEASE_PID}" 2>/dev/null || true
    LEASE_PID=""
  fi
  COLLECT_RUN_ID="${RUN_ID}"
  set +e
  collect_remote "${RUN_ID}"
  status=$?
  set -e
  exit "${status}"
}

disconnect_registered_run() {
  local status
  exec 9>&- || true
  RUN_CONTROL_OPEN=0
  if [ -n "${LEASE_PID}" ]; then
    wait "${LEASE_PID}" 2>/dev/null || true
    LEASE_PID=""
  fi
  COLLECT_RUN_ID="${RUN_ID}"
  set +e
  collect_remote "${RUN_ID}"
  status=$?
  set -e
  exit "${status}"
}

if [ "${COLLECT_MODE}" -eq 1 ]; then
  collect_remote "${COLLECT_RUN_ID}"
  exit $?
fi

# ---------------------------------------------------------------------------
# --sync-only and --pull write only into this checkout, so they take the
# checkout lock and never the physical-maintenance barrier. Durable runs take
# both locks in the owner service; shell and provisioning take the exclusive
# physical-maintenance barrier.
if [ "${SYNC_ONLY}" -eq 1 ] || [ "${PULL_MODE}" -eq 1 ]; then
  HOLDER_LOCK_PATH="${METAL_TREE_LOCK_PATH}"
  HOLDER_OWNER_PATH="${METAL_TREE_OWNER_PATH}"
  HOLDER_UMASK="umask 022; "
  log "acquire metal tree lock (${HOLDER_LOCK_PATH})"
else
  HOLDER_LOCK_PATH="${METAL_LOCK_PATH}"
  HOLDER_OWNER_PATH="${METAL_OWNER_PATH}"
  HOLDER_UMASK=""
  log "acquire exclusive physical-maintenance barrier (${HOLDER_LOCK_PATH})"
fi
# ---------------------------------------------------------------------------
ACTION="bootstrap"
[ "${SYNC_ONLY}" -eq 1 ] && ACTION="sync"
[ "${PULL_MODE}" -eq 1 ] && ACTION="pull"
[ "${RUN_MODE}" -eq 1 ] && ACTION="run"
[ "${SHELL_MODE}" -eq 1 ] && ACTION="shell"
SCENARIO="${OVERDRIVE_METAL_SCENARIO:-unspecified}"
COMMIT="$(git -C "${REPO}" rev-parse HEAD)"
WORKSPACE="${REPO}"
SOURCE_DIGEST="$({
  git -C "${REPO}" diff --binary HEAD
  git -C "${REPO}" ls-files --others --exclude-standard -z \
    | while IFS= read -r -d '' path; do
        printf '%s\0' "${path}"
        shasum -a 256 "${REPO}/${path}"
      done
} | shasum -a 256 | awk '{print $1}')"
EXPECTED_MARKER="commit=${COMMIT}
workspace=${WORKSPACE}
source_digest=${SOURCE_DIGEST}"

LEASE_TMP="$(mktemp -d "${TMPDIR:-/tmp}/overdrive-metal-lease.XXXXXX")"
mkfifo "${LEASE_TMP}/release"
LEASE_HELPER_B64="$(base64 < "${REPO}/infra/metal/lease-holder.sh" | tr -d '\n')"
if [ "${RUN_MODE}" -eq 1 ]; then
  # The durable service owns both locks before the controller transfers source.
  # The service keeps them through command completion; the controller never
  # releases and reacquires the checkout lock around START.
  REMOTE_PATH=""
  JOINED=""
  for arg in "${RUN_ARGS[@]}"; do
    printf -v QUOTED '%q' "${arg}"
    JOINED+="${QUOTED} "
  done
  RUN_EXEC="${JOINED}"
  if [ "${EXCLUSIVE_MODE}" -eq 0 ] && [ "${ORDINARY_SUITE_RUN}" -eq 1 ]; then
    printf -v RUN_EXEC 'bash %q --guest-suite %s' \
      "${REMOTE_DIR}/infra/metal/test-wrapper.sh" "${JOINED}"
  fi
  printf -v REMOTE_CWD 'cd %q' "${REMOTE_DIR}"
  printf -v METAL_RUN_ENV \
    'TMPDIR=%q OVERDRIVE_METAL_RUN_ID=%q OVERDRIVE_METAL_EXECUTION_CLASS=%q OVERDRIVE_METAL_SOURCE_ROOT=%q OVERDRIVE_METAL_TEST_DISPATCHER=%q OVERDRIVE_METAL_KERNEL=%q OVERDRIVE_METAL_ROOTFS=%q' \
    "${METAL_RUN_TMP}" "${RUN_ID}" "${EXECUTION_CLASS}" "${REMOTE_DIR}" \
    "${METAL_TEST_DISPATCHER}" "${OVERDRIVE_METAL_KERNEL:-}" "${OVERDRIVE_METAL_ROOTFS:-}"
  if [ "${NO_SUDO}" -eq 1 ]; then
    INNER="${REMOTE_CWD} && env ${METAL_RUN_ENV} ${RUN_EXEC}; rc=\$?; exit \"\$rc\""
    EXECUTION_MODE="login"
  else
    # The command runs as root inside the rsynced tree, so anything it creates
    # or rewrites (recorded evidence, root-only dirs) would be root-owned and
    # make the login user's next --sync-only rsync fail with "Permission
    # denied". Hand every non-login-owned entry back after the command,
    # whatever its status; /target is excluded by rsync and is huge, so it is
    # pruned. The chown is best effort and silent: it never changes the
    # command's exit status or output.
    INNER="${REMOTE_CWD} && sudo -n env \"HOME=\$HOME\" \"PATH=\$PATH\" ${METAL_RUN_ENV} ${RUN_EXEC}; rc=\$?; sudo -n find ${REMOTE_DIR} -path ${REMOTE_DIR}/target -prune -o ! -user \"\$(id -un)\" -exec chown -h \"\$(id -u):\$(id -g)\" {} + >/dev/null 2>&1 || true; exit \"\$rc\""
    EXECUTION_MODE="privileged"
  fi
  # ExecStopPost runs only after systemd's mixed-mode stop has drained the
  # unit's descendants. Keep this invocation's helper bytes in the unit
  # property so a later source sync cannot replace the finalizer it runs.
  printf -v FINALIZER_COMMAND \
    'echo %s | base64 -d | /bin/bash -s -- --finalize %s %s' \
    "${LEASE_HELPER_B64}" "${RUN_ID}" "${METAL_RUNS_ROOT}"
  printf -v FINALIZER_PROPERTY 'ExecStopPost=/bin/bash -c "%s"' "${FINALIZER_COMMAND}"
  # Decode the helper into bash -c's argument. Every durable argument is
  # quoted by bash's own `%q`; the stop-post command carries this invocation's
  # helper bytes so later checkout sync cannot change its finalizer.
  printf -v REMOTE_LEASE_CMD \
    'lease_script=$(printf %%s %q | base64 -d); exec %s systemd-run --quiet --unit=%q --pipe --wait --collect --expand-environment=no --property=Type=exec --property=KillMode=mixed --property=%q bash -c "$lease_script" overdrive-metal-lease --durable %q %q %q %q %q %q %q %q %q %q %q %q %q %q %q %q %q %q' \
    "${LEASE_HELPER_B64}" "${SUDO}" "overdrive-metal-run-${RUN_ID}.service" \
    "${FINALIZER_PROPERTY}" \
    "${METAL_LOCK_PATH}" "${METAL_OWNER_PATH}" "${METAL_LEASE_TIMEOUT_SECONDS}" \
    "${RUN_ID}" "${SCENARIO}" "${WORKSPACE}" "${COMMIT}" "${SOURCE_DIGEST}" \
    "${REMOTE_DIR}" "${REMOTE_USER}" "${EXECUTION_MODE}" "${REMOTE_HOME}" \
    "${REMOTE_PATH}" "${INNER}" "${METAL_RUNS_ROOT}" \
    "${METAL_TREE_LOCK_PATH}" "${METAL_TREE_OWNER_PATH}" "${EXECUTION_CLASS}"
else
  # Existing sync/shell/bootstrap lease holder: its stdin remains the release
  # stream, with no remote helper file written before the lock exists. For
  # --sync-only the lock is the per-tree one (created 0644 so the durable run
  # command's login user can open it read-only).
  printf -v REMOTE_LEASE_CMD \
    'lease_script=$(printf %%s %q | base64 -d); %sexec %s bash -c "$lease_script" overdrive-metal-lease %q %q %q %q %q %q %q %q' \
    "${LEASE_HELPER_B64}" "${HOLDER_UMASK}" "${SUDO}" "${HOLDER_LOCK_PATH}" "${HOLDER_OWNER_PATH}" \
    "${METAL_LEASE_TIMEOUT_SECONDS}" "${LEASE_TOKEN}" "${ACTION}" "${SCENARIO}" \
    "${WORKSPACE}" "${COMMIT}"
fi
ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_LEASE_CMD}" \
  <"${LEASE_TMP}/release" >"${LEASE_TMP}/status" 2>&1 &
LEASE_PID=$!
exec 9>"${LEASE_TMP}/release"
RUN_CONTROL_OPEN=1

# Poll for acknowledgement no longer than the lease's own acquisition
# window (the remote holder's `flock -w ${METAL_LEASE_TIMEOUT_SECONDS}`)
# plus a small margin, at 0.1s per attempt. Deriving the bound from the
# configured timeout — rather than a fixed 120s-shaped constant — keeps a
# fast-timeout caller (the metal-lease contention test runs three writers
# with OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=1) from spinning here for the
# production default's ~120s if death-detection is ever slow to observe an
# already-timed-out holder under load.
LEASE_POLL_ATTEMPTS=$(( (METAL_LEASE_TIMEOUT_SECONDS + 5) * 10 ))
if [ "${RUN_MODE}" -eq 1 ]; then
  for _attempt in $(seq 1 "${LEASE_POLL_ATTEMPTS}"); do
    if grep -q "OVERDRIVE_METAL_LEASE_ACQUIRED token=${LEASE_TOKEN}" "${LEASE_TMP}/status"; then
      break
    fi
    if ! kill -0 "${LEASE_PID}" 2>/dev/null; then
      exec 9>&- || true
      RUN_CONTROL_OPEN=0
      wait "${LEASE_PID}" 2>/dev/null || true
      cat "${LEASE_TMP}/status" >&2
      COLLECT_RUN_ID="${RUN_ID}"
      set +e
      collect_remote "${RUN_ID}"
      status=$?
      set -e
      exit "${status}"
    fi
    sleep 0.1
  done
  if ! grep -q "OVERDRIVE_METAL_LEASE_ACQUIRED token=${LEASE_TOKEN}" "${LEASE_TMP}/status"; then
    echo "FATAL: durable lease holder did not acknowledge acquisition" >&2
    cat "${LEASE_TMP}/status" >&2
    exec 9>&- || true
    RUN_CONTROL_OPEN=0
    wait "${LEASE_PID}" 2>/dev/null || true
    COLLECT_RUN_ID="${RUN_ID}"
    set +e
    collect_remote "${RUN_ID}"
    status=$?
    set -e
    exit "${status}"
  fi
  cat "${LEASE_TMP}/status"
else
  for _attempt in $(seq 1 "${LEASE_POLL_ATTEMPTS}"); do
    if grep -q "OVERDRIVE_METAL_LEASE_ACQUIRED token=${LEASE_TOKEN}" "${LEASE_TMP}/status"; then
      break
    fi
    if ! kill -0 "${LEASE_PID}" 2>/dev/null; then
      cat "${LEASE_TMP}/status" >&2
      wait "${LEASE_PID}" || true
      exit 75
    fi
    sleep 0.1
  done
  if ! grep -q "OVERDRIVE_METAL_LEASE_ACQUIRED token=${LEASE_TOKEN}" "${LEASE_TMP}/status"; then
    echo "FATAL: lease holder did not acknowledge acquisition" >&2
    cat "${LEASE_TMP}/status" >&2
    exit 75
  fi
  cat "${LEASE_TMP}/status"
fi

# ---------------------------------------------------------------------------
# --pull: the reverse of the sync below, under the same per-tree lock. Copies
# only the named paths from the remote tree into this checkout. No --delete
# (files removed remotely stay locally), `target/` is never transferred, and
# nothing is written to the remote tree.
REMOTE_RSYNC_READY=0
if [ "${SYNC_ONLY}" -eq 1 ] && [ "${NO_SYNC}" -eq 0 ]; then
  ssh "${SSH_OPTS[@]}" "${TARGET}" 'command -v rsync >/dev/null' \
    || rsync_prerequisite_failed
  REMOTE_RSYNC_READY=1
fi
if [ "${RUN_MODE}" -eq 1 ] && [ "${NO_SYNC}" -eq 0 ]; then
  if ssh "${SSH_OPTS[@]}" "${TARGET}" 'command -v rsync >/dev/null'; then
    REMOTE_RSYNC_READY=1
  elif [ "${EXCLUSIVE_MODE}" -eq 0 ]; then
    rsync_prerequisite_failed
  fi
fi

if [ "${PULL_MODE}" -eq 1 ]; then
  ssh "${SSH_OPTS[@]}" "${TARGET}" 'command -v rsync >/dev/null' \
    || rsync_prerequisite_failed
  ensure_checkout_binding check
  for _path in "${RUN_ARGS[@]}"; do
    _path="${_path%/}"
    # shellcheck disable=SC2029
    _kind="$(ssh "${SSH_OPTS[@]}" "${TARGET}" \
      "if [ -d ${REMOTE_DIR}/${_path} ]; then echo dir; elif [ -e ${REMOTE_DIR}/${_path} ]; then echo file; else echo missing; fi")"
    case "${_kind}" in
      dir) _src="${REMOTE_DIR}/${_path}/"; _dst="${REPO}/${_path}/"; mkdir -p "${_dst}" ;;
      file) _src="${REMOTE_DIR}/${_path}"; _dst="${REPO}/${_path}"; mkdir -p "$(dirname "${_dst}")" ;;
      *) echo "FATAL: ${_path} does not exist in ${TARGET}:${REMOTE_DIR}" >&2; exit 1 ;;
    esac
    log "pull ${TARGET}:${REMOTE_DIR}/${_path} -> ${_path}"
    set +e
    "${RSYNC_BIN}" -az --itemize-changes \
      --exclude='target/' \
      -e "ssh ${SSH_OPTS[*]}" \
      "${TARGET}:${_src}" "${_dst}"
    _rc=$?
    set -e
    if [ "${_rc}" -ne 0 ]; then
      echo "FATAL: rsync failed pulling ${_path} (exit ${_rc})" >&2
      exit "${_rc}"
    fi
  done
  log "pull complete (${#RUN_ARGS[@]} path(s); nothing deleted locally)"
  exit 0
fi

if [ "${NO_SYNC}" -eq 1 ]; then
  ensure_checkout_binding check
else
  ensure_checkout_binding bind
fi

if [ "${NO_SYNC}" -eq 0 ]; then
  if [ "${REMOTE_RSYNC_READY}" -eq 0 ] \
    && ! ssh "${SSH_OPTS[@]}" "${TARGET}" 'command -v rsync >/dev/null'; then
    if [ "${SYNC_ONLY}" -eq 1 ] || [ "${PULL_MODE}" -eq 1 ]; then
      rsync_prerequisite_failed
    fi
    log "installing rsync on the remote"
    # shellcheck disable=SC2029
    ssh "${SSH_OPTS[@]}" "${TARGET}" \
      "${SUDO} env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 update -qq && \
       ${SUDO} env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 install -y -qq rsync"
  fi

# ---------------------------------------------------------------------------
log "sync ${REPO} -> ${TARGET}:${REMOTE_DIR}"
# ---------------------------------------------------------------------------
# Excludes: build output only. spike-scratch/ source IS synced (that is the
# point — the probe code is gitignored, so a clone would arrive empty) but its
# target/ and out/ dirs are not: they hold host-arch binaries and a rootfs
# image that would be wrong on the far side anyway.
ssh "${SSH_OPTS[@]}" "${TARGET}" "mkdir -p ${REMOTE_DIR}"
"${RSYNC_BIN}" -az --delete "${STATS_FLAG[@]}" \
  --exclude='/target/' \
  --exclude='spike-scratch/*/target/' \
  --exclude='spike-scratch/*/out/' \
  --exclude='node_modules/' \
  --exclude='/vendors/linux/' \
  --exclude='/.overdrive-metal-checkout.v1' \
  --exclude='.DS_Store' \
  --exclude='/.git' \
  --exclude='/.git/' \
  -e "ssh ${SSH_OPTS[*]}" \
  "${REPO}/" "${TARGET}:${REMOTE_DIR}/"

# ---------------------------------------------------------------------------
# Git — worktree-aware.
#
# This checkout is a git WORKTREE, so `.git` is a one-line pointer FILE:
#     gitdir: /Users/marcus/Git/helios/.git/worktrees/hanoi
# Syncing it verbatim lands a pointer to a path that does not exist on the
# remote, and every git command there fails with an obscure error. So `.git`
# is excluded from the main rsync above; the block below replaces it.
#
# Every sync materialises a real, standalone repo on the box by reassembling
# the two halves a worktree keeps apart:
#   - the COMMON dir (objects, refs, config)  -> becomes the remote .git
#   - this worktree's HEAD/index              -> overlaid on top
# so the remote HEAD and index track this worktree on each sync. Recording
# evidence on the box (e.g. `cargo xtask quint check --record`) needs git there.
# Cost: the common dir is shared by every worktree of this repo (334 MB here),
# so the first sync moves all of it. Incremental afterwards.
# ---------------------------------------------------------------------------
GIT_COMMON="$(cd "${REPO}" && git rev-parse --git-common-dir)"
GIT_WT="$(cd "${REPO}" && git rev-parse --git-dir)"
# `--git-common-dir` can come back relative; normalise both.
GIT_COMMON="$(cd "${REPO}" && cd "${GIT_COMMON}" && pwd)"
GIT_WT="$(cd "${REPO}" && cd "${GIT_WT}" && pwd)"

log "materialising standalone .git (common dir $(du -sh "${GIT_COMMON}" | cut -f1))"
# Exclude worktrees/ — it describes OTHER checkouts on the laptop and is
# meaningless (and misleading) on the remote.
"${RSYNC_BIN}" -az --delete "${STATS_FLAG[@]}" \
  --exclude='/worktrees/' \
  -e "ssh ${SSH_OPTS[*]}" \
  "${GIT_COMMON}/" "${TARGET}:${REMOTE_DIR}/.git/"

# The common dir's HEAD points at the MAIN worktree's branch, not ours.
# Overlay this worktree's HEAD and index so the remote lands on our commit.
"${RSYNC_BIN}" -az \
  -e "ssh ${SSH_OPTS[*]}" \
  "${GIT_WT}/HEAD" "${GIT_WT}/index" \
  "${TARGET}:${REMOTE_DIR}/.git/"

# shellcheck disable=SC2029
ssh "${SSH_OPTS[@]}" "${TARGET}" \
  "git -C ${REMOTE_DIR} config core.bare false && \
   git -C ${REMOTE_DIR} config --unset core.worktree 2>/dev/null; \
   git -C ${REMOTE_DIR} status --short --branch | head -5" || true

SOURCE_MARKER="${REMOTE_DIR}/.overdrive-metal-source"
SOURCE_MARKER_TMP="${SOURCE_MARKER}.tmp.${LEASE_TOKEN}"
printf -v SOURCE_MARKER_COMMAND 'tmp=%q; marker=%q; cat > "$tmp" && mv -f -- "$tmp" "$marker"' \
  "${SOURCE_MARKER_TMP}" "${SOURCE_MARKER}"
printf '%s\n' "${EXPECTED_MARKER}" \
  | ssh "${SSH_OPTS[@]}" "${TARGET}" "${SOURCE_MARKER_COMMAND}"
else
  log "no-sync source identity check"
  ACTUAL_MARKER="$(ssh "${SSH_OPTS[@]}" "${TARGET}" \
    "cat ${REMOTE_DIR}/.overdrive-metal-source 2>/dev/null" || true)"
  if [ "${ACTUAL_MARKER}" != "${EXPECTED_MARKER}" ]; then
    source_mismatch
  fi
fi

if [ "${SYNC_ONLY}" -eq 1 ]; then
  log "sync-only; skipping provisioning"
  exit 0
fi

if [ "${RUN_MODE}" -eq 1 ]; then
  RUN_DIRECTORY="${METAL_RUNS_ROOT}/${RUN_ID}"
  REMOTE_CHECKOUT_BINDING="${REMOTE_DIR}/.overdrive-metal-checkout.v1"
  printf -v REMOTE_CHECKOUT_PREP \
    'set -eu; source=%q; run=%q; [ -f "$source" ] && [ ! -L "$source" ]; [ ! -e "$run/checkout.v1" ] && [ ! -L "$run/checkout.v1" ]; tmp="$run/checkout.v1.tmp.%s"; umask 077; cp -- "$source" "$tmp"; sync -d "$tmp"; mv -f -- "$tmp" "$run/checkout.v1"; sync -f "$run"' \
    "${REMOTE_CHECKOUT_BINDING}" "${RUN_DIRECTORY}" "${RUN_ID}"
  if ssh "${SSH_OPTS[@]}" "${TARGET}" "${SUDO} bash -c $(printf '%q' "${REMOTE_CHECKOUT_PREP}")"; then
    :
  else
    REMOTE_PREP_STATUS=$?
    if [ "${REMOTE_PREP_STATUS}" -eq 255 ]; then
      disconnect_registered_run
    fi
    run_preparation_failed
  fi

  # Keep the common parent writable by the login group while the sticky bit
  # prevents one invocation from unlinking another mode's private run root.
  # The checkout lock also lets login-mode Cargo use only this checkout's
  # default target after an earlier privileged build.
  printf -v REMOTE_TMP_PREP \
    'set -eu; remote=%q; target="$remote/target"; base="$target/metal-run-tmp"; run=%q; user=%q; mode=%q; user_group="$(id -g "$user")"; staging_base=/srv/vm/metal; staging_run="$staging_base/%s"; runtime_base=/run/ovd-gk; runtime_run="$runtime_base/%s"; [ ! -L "$target" ] || exit 1; if [ ! -d "$target" ]; then %s install -d -m 0755 -o "$user" "$target"; fi; [ ! -L "$base" ] || exit 1; %s install -d -m 01730 -o root -g "$user_group" "$base"; [ ! -e "$run" ] && [ ! -L "$run" ] || exit 1; if [ "$mode" = login ]; then [ ! -L "$target/debug" ] || exit 1; if [ -d "$target/debug" ]; then %s chown -R --from=0 "$user" "$target/debug"; fi; fi; if [ "$mode" = privileged ]; then %s install -d -m 0700 -o root "$run"; else %s install -d -m 0700 -o "$user" "$run"; fi; [ ! -L "$staging_base" ] && [ ! -L "$runtime_base" ] || exit 1; [ ! -e "$staging_run" ] && [ ! -L "$staging_run" ] || exit 1; [ ! -e "$runtime_run" ] && [ ! -L "$runtime_run" ] || exit 1; %s install -d -o root -m 0710 "$staging_base"; %s chown 0:4200 "$staging_base"; %s chmod 0710 "$staging_base"; %s install -d -o root -m 0710 "$staging_run"; %s chown 0:4200 "$staging_run"; %s chmod 0710 "$staging_run"; %s install -d -o root -m 0710 "$runtime_base"; %s chown 0:4200 "$runtime_base"; %s chmod 0710 "$runtime_base"; %s install -d -o root -m 0710 "$runtime_run"; %s chown 0:4200 "$runtime_run"; %s chmod 0710 "$runtime_run"' \
    "${REMOTE_DIR}" "${METAL_RUN_TMP}" "${REMOTE_USER}" "${EXECUTION_MODE}" \
    "${RUN_ID}" "${RUN_ID}" \
    "${SUDO}" "${SUDO}" "${SUDO}" "${SUDO:+${SUDO} -u ${REMOTE_USER}}" \
    "${SUDO}" \
    "${SUDO}" "${SUDO}" "${SUDO}" "${SUDO}" "${SUDO}" "${SUDO}" \
    "${SUDO}" "${SUDO}" "${SUDO}" "${SUDO}" "${SUDO}" "${SUDO}"
  if ssh "${SSH_OPTS[@]}" "${TARGET}" "bash -c $(printf '%q' "${REMOTE_TMP_PREP}")"; then
    :
  else
    REMOTE_PREP_STATUS=$?
    if [ "${REMOTE_PREP_STATUS}" -eq 255 ]; then
      disconnect_registered_run
    fi
    run_preparation_failed
  fi

  log "fail-closed native x86_64/KVM preflight"
  printf -v REMOTE_PREFLIGHT_CMD \
    '%s env OVERDRIVE_EXPECTED_TOKEN=%q OVERDRIVE_EXPECTED_COMMIT=%q OVERDRIVE_EXPECTED_WORKSPACE=%q OVERDRIVE_EXPECTED_SOURCE=%q OVERDRIVE_REMOTE_DIR=%q OVERDRIVE_METAL_OWNER_PATH=%q OVERDRIVE_METAL_KERNEL=%q OVERDRIVE_METAL_ROOTFS=%q bash %q' \
    "${SUDO}" "${LEASE_TOKEN}" "${COMMIT}" "${WORKSPACE}" "${SOURCE_DIGEST}" "${REMOTE_DIR}" \
    "${RUN_DIRECTORY}/owner.v1" "${OVERDRIVE_METAL_KERNEL:-}" "${OVERDRIVE_METAL_ROOTFS:-}" \
    "${REMOTE_DIR}/infra/metal/native-preflight.sh"
  PREFLIGHT_STDERR="${LEASE_TMP}/preflight.stderr"
  if ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_PREFLIGHT_CMD}" 2>"${PREFLIGHT_STDERR}"; then
    cat "${PREFLIGHT_STDERR}" >&2
  else
    PREFLIGHT_STATUS=$?
    cat "${PREFLIGHT_STDERR}" >&2
    if grep -q 'OVERDRIVE_METAL_RUNNER_ERROR error=preflight_failed probe=' \
      "${PREFLIGHT_STDERR}"; then
      abort_registered_run preflight_failed
    fi
    [ "${PREFLIGHT_STATUS}" -ne 255 ] || disconnect_registered_run
    run_preparation_failed
  fi

  # START is the one controller word.  Closing the pipe immediately after it
  # lets the service detach all three inherited descriptors and makes an SSH
  # disconnect harmless; it never retries or submits the command again. The
  # service keeps both admission descriptors for the complete command.
  START_SENT=1
  printf 'START\n' >&9 2>/dev/null || true
  exec 9>&- || true
  RUN_CONTROL_OPEN=0

  PHASE_PATH="${METAL_RUNS_ROOT}/${RUN_ID}/phase.v1"
  STARTED=0
  printf -v STARTED_QUERY_BODY \
    'test -f %q && grep -qx %q %q' \
    "${PHASE_PATH}" "phase=running" "${PHASE_PATH}"
  if [ -n "${SUDO}" ]; then
    # Keep this as a login-shell read-only query so it observes the same
    # target-user environment as the pre-start command boundary.  The sudo
    # env wrapper carries the selected guest paths without executing them.
    printf -v STARTED_QUERY_ENV \
      'sudo -n env "HOME=$HOME" "PATH=$PATH" OVERDRIVE_METAL_KERNEL=%q OVERDRIVE_METAL_ROOTFS=%q bash -c %q' \
      "${OVERDRIVE_METAL_KERNEL:-}" "${OVERDRIVE_METAL_ROOTFS:-}" "${STARTED_QUERY_BODY}"
    printf -v REMOTE_STARTED_QUERY 'bash -lc %q' "${STARTED_QUERY_ENV}"
  else
    printf -v REMOTE_STARTED_QUERY 'bash -lc %q' "${STARTED_QUERY_BODY}"
  fi
  STARTED_POLL_ATTEMPTS=$(( (METAL_LEASE_TIMEOUT_SECONDS + 5) * 10 ))
  for _attempt in $(seq 1 "${STARTED_POLL_ATTEMPTS}"); do
    if ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_STARTED_QUERY}" >/dev/null 2>&1; then
      STARTED=1
      break
    fi
    if ! kill -0 "${LEASE_PID}" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
  if [ "${STARTED}" -eq 1 ]; then
    printf 'OVERDRIVE_METAL_RUN_STARTED run_id=%s\n' "${RUN_ID}" >&2
  fi

  set +e
  wait "${LEASE_PID}"
  CONTROLLER_STATUS=$?
  set -e
  if [ "${STARTED}" -eq 0 ] \
    && ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_STARTED_QUERY}" >/dev/null 2>&1; then
    printf 'OVERDRIVE_METAL_RUN_STARTED run_id=%s\n' "${RUN_ID}" >&2
  fi
  COLLECT_RUN_ID="${RUN_ID}"
  set +e
  collect_remote "${RUN_ID}"
  status=$?
  set -e
  exit "${status}"
fi

if [ "${SHELL_MODE}" -eq 1 ]; then
  ssh "${SSH_OPTS[@]}" -t "${TARGET}" "cd ${REMOTE_DIR} && exec \"\$SHELL\" -l"
  exit $?
fi

# ---------------------------------------------------------------------------
log "provision (root)"
# ---------------------------------------------------------------------------
# --build-user is supplied at the call site below (it is REMOTE_USER).
PROV_ARGS=()
[ -n "${BREAK_RAID_DISK}" ] && PROV_ARGS+=(--break-raid-disk "${BREAK_RAID_DISK}")
[ -n "${DATA_DISK}" ] && PROV_ARGS+=(--data-disk "${DATA_DISK}")

# shellcheck disable=SC2029
ssh "${SSH_OPTS[@]}" "${TARGET}" \
  "chmod +x ${REMOTE_DIR}/infra/provision/*.sh ${REMOTE_DIR}/infra/metal/*.sh && \
   ${SUDO} ${REMOTE_DIR}/infra/metal/provision.sh --build-user ${REMOTE_USER} ${PROV_ARGS[*]:-}"

# ---------------------------------------------------------------------------
log "provision (user-mode, as ${REMOTE_USER})"
# ---------------------------------------------------------------------------
# The login user IS the build user — no separate account is created. rustup
# belongs to an unprivileged user, and common-user.sh refuses to run as root,
# so this step is skipped when the login user is root rather than silently
# installing a root-owned toolchain.
if [ "${REMOTE_USER}" = "root" ]; then
  echo "  login user is root; skipping (rustup must not be installed as root)."
  echo "  Log in as an unprivileged user, or run by hand:"
  echo "    ${REMOTE_DIR}/infra/provision/common-user.sh"
else
  # shellcheck disable=SC2029
  ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_DIR}/infra/provision/common-user.sh"
fi

log "done"
cat <<EOF

Next:
  ssh ${TARGET}
  cd ${REMOTE_DIR}

Re-sync after editing probe code locally:
  infra/metal/bootstrap.sh ${TARGET} --sync-only

Open probes (see docs/feature/microvm-driver-cloud-hypervisor/spike/wave-decisions.md):
  P6  virtiofsd + --memory shared=on, in full   <- the one this box exists for
  P4  cp --reflink=auto rootfs copy cost
  P3  the pinned 6.18 kernel under CH
  P1/P2/P5 confirmation, to retire the nested-host caveat
EOF
