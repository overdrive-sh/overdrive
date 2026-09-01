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
# Usage:
#   infra/metal/bootstrap.sh root@1.2.3.4
#   infra/metal/bootstrap.sh root@1.2.3.4 --data-disk /dev/sdb
#   infra/metal/bootstrap.sh root@1.2.3.4 --sync-only
#   infra/metal/bootstrap.sh root@1.2.3.4 --run -- cargo nextest run
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Relative to the login user's HOME, not an absolute /root path. Scaleway's
# Ubuntu images log in as `ubuntu`, not `root` (root's authorized_keys carries
# the stock "please login as ubuntu" forced command), so anything hardcoded
# under /root fails on the first mkdir. The login user IS the build user.
REMOTE_SUBDIR="overdrive"
SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o ServerAliveInterval=30)

[ $# -ge 1 ] || { echo "usage: $0 <user@host> [--data-disk DEV] [--sync-only]" >&2; exit 1; }
TARGET="$1"; shift

DATA_DISK=""
BREAK_RAID_DISK=""
SYNC_ONLY=0
WITH_GIT=0
RUN_MODE=0
SHELL_MODE=0
NO_SYNC=0
NO_SUDO=0
RUN_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --data-disk) DATA_DISK="$2"; shift 2 ;;
    --break-raid-disk) BREAK_RAID_DISK="$2"; shift 2 ;;
    --sync-only) SYNC_ONLY=1; shift ;;
    --with-git)  WITH_GIT=1; shift ;;
    --run) RUN_MODE=1; shift ;;
    --shell) SHELL_MODE=1; shift ;;
    --no-sync) NO_SYNC=1; shift ;;
    --no-sudo) NO_SUDO=1; shift ;;
    --) shift; RUN_ARGS=("$@"); break ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

[ $((SYNC_ONLY + RUN_MODE + SHELL_MODE)) -le 1 ] || {
  echo "FATAL: choose only one of --sync-only, --run, or --shell" >&2
  exit 1
}
[ "${RUN_MODE}" -eq 0 ] || [ "${#RUN_ARGS[@]}" -gt 0 ] || {
  echo "FATAL: --run requires a command after --" >&2
  exit 1
}

METAL_LOCK_PATH="${OVERDRIVE_METAL_LOCK_PATH:-/run/lock/overdrive-metal-shared.lock}"
METAL_OWNER_PATH="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"
METAL_LEASE_TIMEOUT_SECONDS="${OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS:-120}"
LEASE_TOKEN="$(printf '%s-%s-%s' "$$" "$(date +%s)" "${RANDOM}" | shasum -a 256 | cut -c1-24)"
LEASE_TMP=""
LEASE_PID=""

cleanup_lease() {
  if [ -n "${LEASE_TMP}" ]; then
    exec 9>&- 2>/dev/null || true
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

# ---------------------------------------------------------------------------
log "acquire canonical metal lease (${METAL_LOCK_PATH})"
# ---------------------------------------------------------------------------
ACTION="bootstrap"
[ "${SYNC_ONLY}" -eq 1 ] && ACTION="sync"
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

LEASE_TMP="$(mktemp -d "${TMPDIR:-/tmp}/overdrive-metal-lease.XXXXXX")"
mkfifo "${LEASE_TMP}/release"
LEASE_HELPER_B64="$(base64 < "${REPO}/infra/metal/lease-holder.sh" | tr -d '\n')"
# Decode the helper into `bash -c`'s command argument. Its stdin therefore
# remains the SSH session's release stream, with no remote helper file written
# before the canonical lock exists (and no extra fd for sudo to close).
# Positional metadata is quoted by bash's own `%q`, never concatenated raw.
printf -v REMOTE_LEASE_CMD \
  'lease_script=$(printf %%s %q | base64 -d); exec %s bash -c "$lease_script" overdrive-metal-lease %q %q %q %q %q %q %q %q' \
  "${LEASE_HELPER_B64}" "${SUDO}" "${METAL_LOCK_PATH}" "${METAL_OWNER_PATH}" \
  "${METAL_LEASE_TIMEOUT_SECONDS}" "${LEASE_TOKEN}" "${ACTION}" "${SCENARIO}" \
  "${WORKSPACE}" "${COMMIT}"
ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_LEASE_CMD}" \
  <"${LEASE_TMP}/release" >"${LEASE_TMP}/status" 2>&1 &
LEASE_PID=$!
exec 9>"${LEASE_TMP}/release"

# Poll for acknowledgement no longer than the lease's own acquisition
# window (the remote holder's `flock -w ${METAL_LEASE_TIMEOUT_SECONDS}`)
# plus a small margin, at 0.1s per attempt. Deriving the bound from the
# configured timeout — rather than a fixed 120s-shaped constant — keeps a
# fast-timeout caller (the metal-lease contention test runs three writers
# with OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=1) from spinning here for the
# production default's ~120s if death-detection is ever slow to observe an
# already-timed-out holder under load.
LEASE_POLL_ATTEMPTS=$(( (METAL_LEASE_TIMEOUT_SECONDS + 5) * 10 ))
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
# is excluded above, unconditionally.
#
# By default the box simply has no git, which is correct for how it is used:
# edit on the laptop, sync up, run there, commit locally. Nothing in the build
# shells out to git.
#
# `--with-git` materialises a real, standalone repo instead, by reassembling
# the two halves a worktree keeps apart:
#   - the COMMON dir (objects, refs, config)  -> becomes the remote .git
#   - this worktree's HEAD/index              -> overlaid on top
# Cost: the common dir is shared by every worktree of this repo (334 MB here),
# so the first sync moves all of it. Incremental afterwards.
# ---------------------------------------------------------------------------
if [ "${WITH_GIT}" -eq 1 ]; then
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
     git -C ${REMOTE_DIR} status --short --branch | head -5"
fi

SOURCE_MARKER="commit=${COMMIT}
workspace=${WORKSPACE}
source_digest=${SOURCE_DIGEST}"
printf '%s\n' "${SOURCE_MARKER}" | ssh "${SSH_OPTS[@]}" "${TARGET}" \
  "cat > ${REMOTE_DIR}/.overdrive-metal-source"
else
  log "no-sync source identity check"
  EXPECTED_MARKER="commit=${COMMIT}
workspace=${WORKSPACE}
source_digest=${SOURCE_DIGEST}"
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

  JOINED=""
  for arg in "${RUN_ARGS[@]}"; do
    printf -v QUOTED '%q' "${arg}"
    JOINED+="${QUOTED} "
  done
  if [ "${NO_SUDO}" -eq 1 ]; then
    INNER="cd ${REMOTE_DIR} && ${JOINED}"
  else
    INNER="cd ${REMOTE_DIR} && sudo -E env \"HOME=\$HOME\" \"PATH=\$PATH\" ${JOINED}"
  fi
  printf -v REMOTE_RUN 'bash -lc %q' "${INNER}"
  ssh "${SSH_OPTS[@]}" "${TARGET}" "${REMOTE_RUN}"
  exit $?
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
