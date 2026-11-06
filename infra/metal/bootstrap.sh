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
# Locking model (two lock levels, always taken global -> tree):
#   * GLOBAL lease (/run/lock/overdrive-metal-shared.lock): one holder per box.
#     --run, --shell and bootstrap (provisioning) take it, because they mutate
#     node-global kernel state (nft tables, routes, cgroups, netns, KVM).
#   * TREE lock (/run/lock/overdrive-metal-tree-<tree>.lock): one holder per
#     branch tree. --sync-only and --pull take ONLY this lock, since they touch only
#     the branch's private tree, so it never waits behind another workspace's
#     run. --run also takes it (after the global lease) around its own rsync,
#     and the durable run command re-takes it for the whole command, so a
#     concurrent sync of the same tree is excluded even after the controller
#     disconnects. See the run-mode comments below.
#
# Usage:
#   infra/metal/bootstrap.sh root@1.2.3.4
#   infra/metal/bootstrap.sh root@1.2.3.4 --data-disk /dev/sdb
#   infra/metal/bootstrap.sh root@1.2.3.4 --sync-only
#   infra/metal/bootstrap.sh root@1.2.3.4 --run -- cargo nextest run
#   infra/metal/bootstrap.sh root@1.2.3.4 --collect <RUN_ID>
#   infra/metal/bootstrap.sh root@1.2.3.4 --pull -- specs/quint/<subsystem>/evidence
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Relative to the login user's HOME, not an absolute /root path. Scaleway's
# Ubuntu images log in as `ubuntu`, not `root` (root's authorized_keys carries
# the stock "please login as ubuntu" forced command), so anything hardcoded
# under /root fails on the first mkdir. The login user IS the build user.
#
# Each branch syncs into its own tree, so parallel workspaces on the shared
# box never rsync over (or --delete inside) another session's tree. The
# directory is the current branch name with every character outside
# [A-Za-z0-9._-] (notably `/`) replaced by `-`; a detached HEAD falls back to
# the local workspace directory name. OVERDRIVE_METAL_REMOTE_SUBDIR overrides
# the whole subdirectory (relative to the remote HOME). Each tree keeps its own
# cargo target dir, so a branch's first run on the box builds cold.
if [ -n "${OVERDRIVE_METAL_REMOTE_SUBDIR:-}" ]; then
  REMOTE_SUBDIR="${OVERDRIVE_METAL_REMOTE_SUBDIR}"
else
  BRANCH="$(git -C "${REPO}" symbolic-ref --quiet --short HEAD 2>/dev/null || basename "${REPO}")"
  REMOTE_SUBDIR="overdrive-branches/$(printf '%s' "${BRANCH}" | tr -c 'A-Za-z0-9._-' '-')"
fi
case "${REMOTE_SUBDIR}" in
  ""|/*|*..*) echo "FATAL: invalid remote subdirectory '${REMOTE_SUBDIR}'" >&2; exit 64 ;;
esac
[[ "${REMOTE_SUBDIR}" =~ ^[A-Za-z0-9._/-]+$ ]] || {
  echo "FATAL: remote subdirectory '${REMOTE_SUBDIR}' has characters outside [A-Za-z0-9._/-]" >&2
  exit 64
}
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)

[ $# -ge 1 ] || { echo "usage: $0 <user@host> [--data-disk DEV] [--sync-only | --pull -- PATH... | --run -- CMD... | --shell | --collect ID]" >&2; exit 1; }
TARGET="$1"; shift

DATA_DISK=""
BREAK_RAID_DISK=""
SYNC_ONLY=0
PULL_MODE=0
RUN_MODE=0
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

RUN_ID=""
if [ "${RUN_MODE}" -eq 1 ]; then
  RUN_ID="$(printf '%s-%s-%s' "$$" "$(date +%s%N)" "${RANDOM}" | shasum -a 256 | cut -c1-24)"
  printf 'OVERDRIVE_METAL_RUN_ID=%s\n' "${RUN_ID}" >&2
fi

METAL_LOCK_PATH="${OVERDRIVE_METAL_LOCK_PATH:-/run/lock/overdrive-metal-shared.lock}"
METAL_OWNER_PATH="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"
METAL_LEASE_TIMEOUT_SECONDS="${OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS:-120}"
# Per-tree lock: named after the remote tree (REMOTE_SUBDIR with `/` -> `-`).
# Both paths are overridable, mirroring the global pair, for test fixtures.
TREE_NAME="${REMOTE_SUBDIR//\//-}"
METAL_TREE_LOCK_PATH="${OVERDRIVE_METAL_TREE_LOCK_PATH:-/run/lock/overdrive-metal-tree-${TREE_NAME}.lock}"
METAL_TREE_OWNER_PATH="${OVERDRIVE_METAL_TREE_OWNER_PATH:-/run/lock/overdrive-metal-tree-${TREE_NAME}.owner}"
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
# Resolve the remote home and whether we need sudo. Both vary by image:
# root on some, `ubuntu` on Scaleway's.
REMOTE_HOME="$(ssh "${SSH_OPTS[@]}" "${TARGET}" 'echo $HOME')"
REMOTE_DIR="${REMOTE_HOME%/}/${REMOTE_SUBDIR}"
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

collect_remote() {
  local helper_b64 remote_command status
  helper_b64="$(base64 < "${REPO}/infra/metal/lease-holder.sh" | tr -d '\n')"
  # Decode the repository-local collector into bash -c's command argument.
  # The collector itself validates the run id again before deriving any unit
  # or path, and it never opens the canonical lease.
  printf -v remote_command \
    'lease_script=$(printf %%s %q | base64 -d); exec %s bash -c "$lease_script" overdrive-metal-lease --collect %q %q %q %q' \
    "${helper_b64}" "${SUDO}" "${COLLECT_RUN_ID:-$1}" \
    "${METAL_LEASE_TIMEOUT_SECONDS}" "${METAL_LOCK_PATH}" "${METAL_OWNER_PATH}"
  set +e
  ssh "${SSH_OPTS[@]}" "${TARGET}" "${remote_command}"
  status=$?
  set -e
  return "${status}"
}

if [ "${COLLECT_MODE}" -eq 1 ]; then
  collect_remote "${COLLECT_RUN_ID}"
  exit $?
fi

# ---------------------------------------------------------------------------
# --sync-only writes only into this branch's private tree, so it takes the
# per-tree lock INSTEAD of the global lease; every other action takes the
# global lease (run additionally takes the tree lock, below).
if [ "${SYNC_ONLY}" -eq 1 ] || [ "${PULL_MODE}" -eq 1 ]; then
  HOLDER_LOCK_PATH="${METAL_TREE_LOCK_PATH}"
  HOLDER_OWNER_PATH="${METAL_TREE_OWNER_PATH}"
  HOLDER_UMASK="umask 022; "
  log "acquire metal tree lock (${HOLDER_LOCK_PATH})"
else
  HOLDER_LOCK_PATH="${METAL_LOCK_PATH}"
  HOLDER_OWNER_PATH="${METAL_OWNER_PATH}"
  HOLDER_UMASK=""
  log "acquire canonical metal lease (${HOLDER_LOCK_PATH})"
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
  # A durable run service needs the login user's PATH and HOME explicitly:
  # the transient service is root-owned, while the command still starts in
  # the target user's login shell before the existing sudo transition.
  # The supervisor resolves the login PATH after it owns the canonical lease;
  # no extra SSH mutation/query is performed while a writer is still waiting.
  REMOTE_PATH=""
  JOINED=""
  for arg in "${RUN_ARGS[@]}"; do
    printf -v QUOTED '%q' "${arg}"
    JOINED+="${QUOTED} "
  done
  # The durable command re-takes the per-tree lock for its whole lifetime and,
  # under that lock, re-verifies the synced source marker before running the
  # user command. The controller releases its own tree lock just before START,
  # and the run outlives the controller's ssh session, so this guard is what
  # keeps a later sync of the same tree out for the run's full duration; a
  # sync that slips into the release->reacquire window changes the marker and
  # the run fails closed. fd 7 is opened read-only (no O_CREAT: the file was
  # created by the controller's tree holder) and stays open in this login
  # shell, so the command must not be exec'd: it runs, then its status is
  # returned.
  printf -v TREE_GUARD \
    'exec 7<%q && flock -w %q 7 || { echo "FATAL: could not acquire metal tree lock %s" >&2; exit 75; }; [ "$(cat %q/.overdrive-metal-source 2>/dev/null)" = %q ] || { echo "FATAL: metal source marker changed before the run started; refusing to run against a different tree" >&2; exit 1; }; ' \
    "${METAL_TREE_LOCK_PATH}" "${METAL_LEASE_TIMEOUT_SECONDS}" "${METAL_TREE_LOCK_PATH}" \
    "${REMOTE_DIR}" "${EXPECTED_MARKER}"
  if [ "${NO_SUDO}" -eq 1 ]; then
    INNER="${TREE_GUARD}cd ${REMOTE_DIR} && ${JOINED}; rc=\$?; exit \"\$rc\""
    EXECUTION_MODE="login"
  else
    printf -v METAL_RUN_ENV \
      'OVERDRIVE_METAL_KERNEL=%q OVERDRIVE_METAL_ROOTFS=%q' \
      "${OVERDRIVE_METAL_KERNEL:-}" "${OVERDRIVE_METAL_ROOTFS:-}"
    # The command runs as root inside the rsynced tree, so anything it creates
    # or rewrites (recorded evidence, root-only dirs) would be root-owned and
    # make the login user's next --sync-only rsync fail with "Permission
    # denied". Hand every non-login-owned entry back after the command,
    # whatever its status; /target is excluded by rsync and is huge, so it is
    # pruned. The chown is best effort and silent: it never changes the
    # command's exit status or output.
    INNER="${TREE_GUARD}cd ${REMOTE_DIR} && sudo -n env \"HOME=\$HOME\" \"PATH=\$PATH\" ${METAL_RUN_ENV} ${JOINED}; rc=\$?; sudo -n find ${REMOTE_DIR} -path ${REMOTE_DIR}/target -prune -o ! -user \"\$(id -un)\" -exec chown -h \"\$(id -u):\$(id -g)\" {} + >/dev/null 2>&1 || true; exit \"\$rc\""
    EXECUTION_MODE="privileged"
  fi
  # Decode the helper into bash -c's argument. Every durable argument is
  # quoted by bash's own `%q`; systemd-run receives the exact command string
  # and --expand-environment=no prevents it from interpreting its dollar
  # signs.
  printf -v REMOTE_LEASE_CMD \
    'lease_script=$(printf %%s %q | base64 -d); exec %s systemd-run --quiet --unit=%q --pipe --wait --collect --expand-environment=no --property=Type=exec --property=KillMode=mixed bash -c "$lease_script" overdrive-metal-lease --durable %q %q %q %q %q %q %q %q %q %q %q %q %q %q %q' \
    "${LEASE_HELPER_B64}" "${SUDO}" "overdrive-metal-run-${RUN_ID}.service" \
    "${METAL_LOCK_PATH}" "${METAL_OWNER_PATH}" "${METAL_LEASE_TIMEOUT_SECONDS}" \
    "${RUN_ID}" "${SCENARIO}" "${WORKSPACE}" "${COMMIT}" "${SOURCE_DIGEST}" \
    "${REMOTE_DIR}" "${REMOTE_USER}" "${EXECUTION_MODE}" "${REMOTE_HOME}" \
    "${REMOTE_PATH}" "${INNER}" "${METAL_RUNS_ROOT}"
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

  # Per-tree lock, taken AFTER the global lease (lock order is always
  # global -> tree; --sync-only takes only the tree lock, so no cycle). It
  # excludes a concurrent sync of this tree while this controller rsyncs and
  # writes the source marker. Nothing mutates before it is held. The holder
  # ssh is started with fd 9 closed so it cannot keep the global release
  # stream open.
  TREE_TOKEN="${LEASE_TOKEN}-tree"
  mkfifo "${LEASE_TMP}/tree-release"
  printf -v TREE_LEASE_CMD \
    'lease_script=$(printf %%s %q | base64 -d); umask 022; exec %s bash -c "$lease_script" overdrive-metal-lease %q %q %q %q %q %q %q %q' \
    "${LEASE_HELPER_B64}" "${SUDO}" "${METAL_TREE_LOCK_PATH}" "${METAL_TREE_OWNER_PATH}" \
    "${METAL_LEASE_TIMEOUT_SECONDS}" "${TREE_TOKEN}" "${ACTION}" "${SCENARIO}" \
    "${WORKSPACE}" "${COMMIT}"
  log "acquire metal tree lock (${METAL_TREE_LOCK_PATH})"
  ssh "${SSH_OPTS[@]}" "${TARGET}" "${TREE_LEASE_CMD}" \
    <"${LEASE_TMP}/tree-release" >"${LEASE_TMP}/tree-status" 2>&1 9>&- &
  TREE_PID=$!
  exec 6>"${LEASE_TMP}/tree-release"
  for _attempt in $(seq 1 "${LEASE_POLL_ATTEMPTS}"); do
    if grep -q "OVERDRIVE_METAL_LEASE_ACQUIRED token=${TREE_TOKEN}" "${LEASE_TMP}/tree-status"; then
      break
    fi
    if ! kill -0 "${TREE_PID}" 2>/dev/null; then
      cat "${LEASE_TMP}/tree-status" >&2
      exit 75
    fi
    sleep 0.1
  done
  if ! grep -q "OVERDRIVE_METAL_LEASE_ACQUIRED token=${TREE_TOKEN}" "${LEASE_TMP}/tree-status"; then
    echo "FATAL: tree lock holder did not acknowledge acquisition" >&2
    cat "${LEASE_TMP}/tree-status" >&2
    exit 75
  fi
  cat "${LEASE_TMP}/tree-status"
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
if [ "${PULL_MODE}" -eq 1 ]; then
  ssh "${SSH_OPTS[@]}" "${TARGET}" 'command -v rsync >/dev/null' || {
    echo "FATAL: rsync is not installed on ${TARGET}; run \`cargo xtask metal sync\` once to install it" >&2
    exit 1
  }
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

if [ "${NO_SYNC}" -eq 0 ]; then
ssh "${SSH_OPTS[@]}" "${TARGET}" 'command -v rsync >/dev/null' || {
  log "installing rsync on the remote"
  # shellcheck disable=SC2029
  ssh "${SSH_OPTS[@]}" "${TARGET}" \
    "${SUDO} env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 update -qq && \
     ${SUDO} env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=600 install -y -qq rsync"
}

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

printf '%s\n' "${EXPECTED_MARKER}" | ssh "${SSH_OPTS[@]}" "${TARGET}" \
  "cat > ${REMOTE_DIR}/.overdrive-metal-source"
else
  log "no-sync source identity check"
  ACTUAL_MARKER="$(ssh "${SSH_OPTS[@]}" "${TARGET}" \
    "cat ${REMOTE_DIR}/.overdrive-metal-source 2>/dev/null" || true)"
  if [ "${ACTUAL_MARKER}" != "${EXPECTED_MARKER}" ]; then
    echo "FATAL: --no-sync refused stale or mismatched metal source" >&2
    echo "expected:" >&2
    printf '%s\n' "${EXPECTED_MARKER}" | sed 's/^/  /' >&2
    echo "actual:" >&2
    printf '%s\n' "${ACTUAL_MARKER:-<missing>}" | sed 's/^/  /' >&2
    exit 1
  fi
fi

if [ "${SYNC_ONLY}" -eq 1 ]; then
  log "sync-only; skipping provisioning"
  exit 0
fi

if [ "${RUN_MODE}" -eq 1 ]; then
  log "fail-closed native x86_64/KVM preflight"
  printf -v REMOTE_PREFLIGHT_CMD \
    '%s env OVERDRIVE_EXPECTED_TOKEN=%q OVERDRIVE_EXPECTED_COMMIT=%q OVERDRIVE_EXPECTED_WORKSPACE=%q OVERDRIVE_EXPECTED_SOURCE=%q OVERDRIVE_REMOTE_DIR=%q OVERDRIVE_METAL_OWNER_PATH=%q OVERDRIVE_METAL_KERNEL=%q OVERDRIVE_METAL_ROOTFS=%q bash %q' \
    "${SUDO}" "${LEASE_TOKEN}" "${COMMIT}" "${WORKSPACE}" "${SOURCE_DIGEST}" "${REMOTE_DIR}" \
    "${METAL_OWNER_PATH}" "${OVERDRIVE_METAL_KERNEL:-}" "${OVERDRIVE_METAL_ROOTFS:-}" \
    "${REMOTE_DIR}/infra/metal/native-preflight.sh"
  ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_PREFLIGHT_CMD}"

  # START is the one controller word.  Closing the pipe immediately after it
  # lets the service detach all three inherited descriptors and makes an SSH
  # disconnect harmless; it never retries or submits the command again.
  # The tree lock is released only now, after the preflight; the durable
  # command re-takes it and re-verifies the marker (see TREE_GUARD).
  release_tree_lease
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
