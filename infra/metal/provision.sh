#!/usr/bin/env bash
# Bare-metal provisioning. Runs ON the box, as root, via infra/metal/bootstrap.sh.
#
# Everything shared with the Lima dev VM lives in infra/provision/ and is
# invoked from here. Only genuinely metal-specific work belongs in this file:
# real disks, real /dev/kvm permissions, and a real unprivileged VMM identity.
#
# Idempotent: safe to re-run against a live box.
#
# Usage:  sudo infra/metal/provision.sh [--data-disk /dev/sdb]
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_USER="${BUILD_USER:-$(logname 2>/dev/null || echo root)}"
DATA_DISK=""          # empty = skip storage setup entirely (see below)
VM_DATA_DIR="/srv/vm"
PROBE_USER="ovdvmm"   # the unprivileged identity the VMM runs as

BREAK_RAID_DISK=""    # empty = never touch the RAID

while [ $# -gt 0 ]; do
  case "$1" in
    --data-disk) DATA_DISK="$2"; shift 2 ;;
    --build-user) BUILD_USER="$2"; shift 2 ;;
    --break-raid-disk) BREAK_RAID_DISK="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

log() { printf '\n########## [metal] %s\n' "$*"; }
[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

# ---------------------------------------------------------------------------
log "host facts — every probe verdict gets pinned to these"
# ---------------------------------------------------------------------------
# Scaleway substitutes hardware behind "or equivalent", so record what we
# ACTUALLY got. A verdict about older hardware is worthless without knowing
# which older hardware produced it.
uname -r
uname -m
grep -m1 '^model name' /proc/cpuinfo || true
grep -cE '^processor' /proc/cpuinfo | sed 's/^/logical cpus: /'
free -h | head -2
lsblk -dno NAME,SIZE,ROTA,MODEL || true

# Bare metal must have real virtualization extensions and NO nesting.
# x86 exposes vmx/svm in /proc/cpuinfo flags; aarch64 has no `flags` line at
# all, so this check must not run there or it fails on hardware that runs KVM
# fine. /dev/kvm presence is the portable signal.
if [ "$(uname -m)" = "x86_64" ]; then
  if ! grep -qE '^flags.*\b(vmx|svm)\b' /proc/cpuinfo; then
    echo "FATAL: no vmx/svm in /proc/cpuinfo — this host cannot run KVM" >&2
    exit 1
  fi
fi
[ -c /dev/kvm ] || {
  echo "FATAL: /dev/kvm missing — kvm module not loaded, or virtualization" >&2
  echo "       disabled in firmware. Cloud Hypervisor cannot run here." >&2
  exit 1
}
grep -qE '^flags.*\bvmx\b' /proc/cpuinfo && echo "virt: Intel VT-x"
grep -qE '^flags.*\bsvm\b' /proc/cpuinfo && echo "virt: AMD-V"
if command -v systemd-detect-virt >/dev/null 2>&1; then
  V="$(systemd-detect-virt || true)"
  echo "systemd-detect-virt: ${V:-none}"
  # The whole reason this box exists is to escape nested virt. If we are
  # nested, say so loudly rather than reproducing the Lima flakiness silently.
  if [ -n "${V}" ] && [ "${V}" != "none" ]; then
    echo "WARNING: this host reports itself virtualized (${V}) — NOT bare metal." >&2
    echo "         microVM boot results from here inherit the same trust problem" >&2
    echo "         as the Lima dev VM. See spike/findings.md." >&2
  fi
fi

# ---------------------------------------------------------------------------
log "media sanity — fail fast on bad hardware"
# ---------------------------------------------------------------------------
# Added 2026-08-10 after a box shipped with failing media. The symptom was NOT
# obvious: the OS booted, networked, and accepted logins, but individual files
# returned EIO while their neighbours in the same directory read fine, and
# binaries took SIGBUS as the page cache drained. Hours went into chasing the
# partition layout, the SSH key, and Scaleway's console before anyone read a
# raw sector. Sixty seconds of surface read would have settled it immediately.
#
# This is not a full surface scan — it samples. A clean result does not prove
# healthy media; a dirty one proves the opposite and is worth aborting on.
# O_DIRECT is PREFERRED (it bypasses the page cache, so we read real media
# rather than something already cached) but it is NOT universally supported —
# on a WDC PC SA530 under kernel 7.0 it returns EINVAL for a read that succeeds
# perfectly well buffered. Treating that as bad media reports a HEALTHY machine
# as broken and tells the operator to return it. Verified 2026-08-10; the first
# version of this check did exactly that on a good box.
#
# So: probe whether O_DIRECT works at all, and fall back to buffered reads with
# the cache dropped first. Only a genuine read failure counts as bad media.
_read_probe() {  # <device> <count-MiB> [skip-MiB]
  local d="$1" cnt="$2" skip="${3:-0}" err
  if [ "${DIRECT_OK}" = "1" ]; then
    err="$(dd if="$d" of=/dev/null bs=1M count="$cnt" skip="$skip" \
              iflag=direct status=none 2>&1)" && return 0
  fi
  sync; echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true
  err="$(dd if="$d" of=/dev/null bs=1M count="$cnt" skip="$skip" status=none 2>&1)" && return 0
  LAST_ERR="$err"; return 1
}

FIRST_DISK="$(lsblk -dno NAME,TYPE | awk '$2=="disk"{print $1; exit}')"
DIRECT_OK=1
if ! dd if="/dev/${FIRST_DISK}" of=/dev/null bs=1M count=1 iflag=direct status=none 2>/dev/null; then
  DIRECT_OK=0
  echo "  note: O_DIRECT unsupported here — using buffered reads with cache drops"
fi

for dev in $(lsblk -dno NAME,TYPE | awk '$2=="disk"{print $1}'); do
  d="/dev/${dev}"
  printf '  %-14s ' "${d}"
  # Read the first and last 512 MiB. Bad blocks cluster at neither end
  # particularly, but a device that cannot serve its own extremes is done.
  SZ=$(blockdev --getsize64 "${d}" 2>/dev/null || echo 0)
  if [ "${SZ}" -eq 0 ]; then echo "unreadable size — SKIP"; continue; fi
  TAIL_SKIP=$(( SZ / 1048576 - 512 )); [ "${TAIL_SKIP}" -lt 0 ] && TAIL_SKIP=0
  if ! _read_probe "${d}" 512 0; then
    echo "READ ERROR at head — BAD MEDIA: ${LAST_ERR}"; MEDIA_BAD=1; continue
  fi
  if ! _read_probe "${d}" 512 "${TAIL_SKIP}"; then
    echo "READ ERROR at tail — BAD MEDIA: ${LAST_ERR}"; MEDIA_BAD=1; continue
  fi
  echo "head+tail readable"
done

# SMART, where the controller exposes it. Reallocated/pending sectors are the
# direct machine-readable version of what cost a day here.
if command -v smartctl >/dev/null 2>&1; then
  for dev in $(lsblk -dno NAME,TYPE | awk '$2=="disk"{print $1}'); do
    smartctl -H "/dev/${dev}" 2>/dev/null | grep -iE "SMART overall|result" || true
    smartctl -A "/dev/${dev}" 2>/dev/null \
      | grep -iE "Reallocated_Sector|Current_Pending|Offline_Uncorrectable|Media_Wearout" || true
  done
else
  echo "  (smartmontools not installed yet — installed below; re-run to get SMART)"
fi

# WRITE test. The read probes above are necessary but NOT sufficient — a disk
# whose head and tail read perfectly can still fail every write, which is
# exactly what happened on 2026-08-10: reads passed, then apt died with
# `I/O error, dev sda, sector … op 0x1:(WRITE)` -> aborted journal -> root
# remounted read-only. Provisioning is write-heavy, so a read-only health check
# gives false confidence precisely where it matters.
#
# Safe by construction: a normal file on the existing root fs (fsync'd, read
# back, removed). No raw writes to a mounted device.
printf '  %-14s ' "write+fsync /"
_WT="$(mktemp /var/tmp/.mediawrite.XXXXXX 2>/dev/null || echo '')"
if [ -z "${_WT}" ]; then
  echo "CANNOT CREATE TEMP FILE — filesystem already read-only?"; MEDIA_BAD=1
else
  if dd if=/dev/urandom of="${_WT}" bs=1M count=64 conv=fsync status=none 2>/dev/null \
     && [ "$(stat -c %s "${_WT}")" = "$((64*1024*1024))" ] \
     && dd if="${_WT}" of=/dev/null bs=1M status=none 2>/dev/null; then
    echo "ok"
  else
    echo "WRITE FAILED — BAD MEDIA (check dmesg for 'op 0x1:(WRITE)')"; MEDIA_BAD=1
  fi
  rm -f "${_WT}"
fi

# Raw write test on the data disk — safe because we are about to mkfs it.
if [ -n "${DATA_DISK}" ] && [ -b "${DATA_DISK}" ] \
   && ! findmnt -S "${DATA_DISK}" >/dev/null 2>&1; then
  printf '  %-14s ' "write ${DATA_DISK}"
  if dd if=/dev/zero of="${DATA_DISK}" bs=1M count=64 conv=fsync status=none 2>/dev/null; then
    echo "ok"
  else
    echo "WRITE FAILED — BAD MEDIA"; MEDIA_BAD=1
  fi
fi

# Anything the kernel already logged beats anything we can probe for.
if dmesg 2>/dev/null | grep -qiE "I/O error, dev |Detected aborted journal|Remounting filesystem read-only|critical medium error"; then
  echo "  !! kernel has ALREADY logged storage errors:"
  dmesg 2>/dev/null | grep -iE "I/O error, dev |aborted journal|Remounting filesystem read-only" | tail -5 | sed 's/^/     /'
  MEDIA_BAD=1
fi

if [ "${MEDIA_BAD:-0}" = "1" ]; then
  echo >&2
  echo "FATAL: a disk failed a basic read. Do NOT provision this machine." >&2
  echo "       Request a replacement rather than reinstalling — a reinstall" >&2
  echo "       simply writes the OS back onto failing media." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
log "shared provisioning (infra/provision)"
# ---------------------------------------------------------------------------
"${REPO}/infra/provision/common-system.sh"

# ---------------------------------------------------------------------------
log "/dev/kvm access — DELIBERATELY NOT the Lima 0666 rule"
# ---------------------------------------------------------------------------
# infra/lima/overdrive-dev.yaml installs a udev rule setting /dev/kvm to 0666
# so any user can open it. We do NOT replicate that here, on purpose.
#
# That rule is exactly what made P5's uid question ambiguous: an unprivileged
# VMM could open /dev/kvm only because the mode was world-writable, which tells
# you nothing about a production host. P5 then proved the realistic shape works
# — an unprivileged uid in the `kvm` group against /dev/kvm at 0660 root:kvm.
# This box keeps the distro default so that result stays honest.
ls -l /dev/kvm
if [ "$(stat -c '%a' /dev/kvm)" = "666" ]; then
  echo "WARNING: /dev/kvm is 0666 on this host; P5's uid evidence will be vacuous." >&2
fi

log "unprivileged VMM identity: ${PROBE_USER}"
getent group kvm >/dev/null || groupadd --system kvm
if ! id -u "${PROBE_USER}" >/dev/null 2>&1; then
  useradd --system --no-create-home --shell /usr/sbin/nologin -G kvm "${PROBE_USER}"
else
  usermod -aG kvm "${PROBE_USER}"
fi
id "${PROBE_USER}"

# The BUILD USER needs /dev/kvm too — it is the account that actually runs the
# probes (bootstrap.sh's --sync-only loop, then `cargo xtask`/run.sh by hand).
# Ubuntu's default cloud user is NOT in the kvm group, and we deliberately keep
# /dev/kvm at 0660 root:kvm rather than the Lima 0666 rule, so without this the
# FIRST cloud-hypervisor launch fails EACCES — looking like a CH bug, which is
# precisely the misattribution this box exists to eliminate.
if [ "${BUILD_USER}" != "root" ] && id -u "${BUILD_USER}" >/dev/null 2>&1; then
  usermod -aG kvm "${BUILD_USER}"
  echo "  ${BUILD_USER} added to kvm: $(id -nG "${BUILD_USER}")"
  echo "  NOTE: group membership needs a NEW login — the current ssh session"
  echo "        does not have it. Reconnect before running probes."
fi

# Prove it by ACTUALLY OPENING the device — this is the P5 precondition.
#
# Do NOT use `test -r` / `test -w`. Those call access(2), which disagrees with
# reality here: on this box `test -r /dev/kvm` as ovdvmm returns FALSE while
# `open(/dev/kvm, O_RDWR)` from the same user SUCCEEDS. access(2) is an
# advisory pre-check and is exactly the call the TOCTOU guidance says never to
# gate an open on. The first version of this check used it and FATAL'd on a
# correctly-configured machine. Verified 2026-08-10.
if runuser -u "${PROBE_USER}" -- sh -c 'exec 3<>/dev/kvm' 2>/dev/null; then
  echo "+++ ${PROBE_USER} opened /dev/kvm O_RDWR (group membership, no 0666 needed)"
else
  echo "FATAL: ${PROBE_USER} cannot open /dev/kvm — P5 confinement cannot run" >&2
  echo "       groups: $(id -nG "${PROBE_USER}")   perms: $(stat -c '%A %U:%G' /dev/kvm)" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Optional: reclaim a disk from Scaleway's default RAID1.
#
# Scaleway's default Elastic Metal layout mirrors BOTH disks (md0=/boot,
# md1=/) and its installer only accepts ext4/fat32 — it rejects `"format":
# "xfs"` outright. That leaves no free device for the reflink-capable data
# volume P4 needs.
#
# Preferred fix is the custom partition JSON (raids: [], sdb1 left
# unformatted). Where that is rejected, this reclaims the second mirror leg
# after the fact: fail+remove its partitions from every md array, then hand
# the whole disk to --data-disk. The arrays continue degraded on the surviving
# disk, which is the correct trade for a throwaway probe box.
#
# Destructive and irreversible, so it is opt-in and heavily guarded.
# ---------------------------------------------------------------------------
if [ -n "${BREAK_RAID_DISK}" ]; then
  log "reclaiming ${BREAK_RAID_DISK} from RAID"
  [ -b "${BREAK_RAID_DISK}" ] || { echo "FATAL: ${BREAK_RAID_DISK} is not a block device" >&2; exit 1; }

  # Guard: never touch the disk the live root is actually served from.
  #
  # Resolve parent disks with lsblk PKNAME, NOT by sed'ing trailing digits.
  # The sed approach is silently WRONG on NVMe: /dev/nvme0n1p2 -> /dev/nvme0n1p,
  # which matches no real device, so the "would this orphan root?" check below
  # passes when it must refuse. What then saved the root disk was util-linux
  # refusing O_EXCL on a claimed device — luck, not a guard.
  ROOT_SRC="$(findmnt -no SOURCE / || true)"
  ROOT_DISKS=""
  if [[ "${ROOT_SRC}" == /dev/md* ]]; then
    for m in $(mdadm --detail "${ROOT_SRC}" 2>/dev/null \
               | awk '/\/dev\/(sd|nvme|vd)/ {print $NF}'); do
      ROOT_DISKS="${ROOT_DISKS}/dev/$(lsblk -no PKNAME "${m}" 2>/dev/null | head -1)"$'\n'
    done
  else
    ROOT_DISKS="/dev/$(lsblk -no PKNAME "${ROOT_SRC}" 2>/dev/null | head -1)"
  fi
  ROOT_DISKS="$(echo "${ROOT_DISKS}" | grep -v '^/dev/$' | sort -u)"
  echo "root is on ${ROOT_SRC} (disks: ${ROOT_DISKS//$'\n'/ })"

  # Refuse if removing this disk would leave root with no backing device.
  REMAINING="$(echo "${ROOT_DISKS}" | grep -v "^${BREAK_RAID_DISK}$" || true)"
  if [ -z "${REMAINING}" ]; then
    echo "FATAL: ${BREAK_RAID_DISK} is the ONLY disk backing root — refusing" >&2
    exit 1
  fi

  # Release ANY active swap on the target disk first. Scaleway's default layout
  # puts a swap partition on the second disk; it is not an md member, so the
  # fail/remove loop skips it, and the device stays claimed. wipefs then fails
  # EBUSY — silently, under `|| true` — and the script cheerfully reports the
  # disk as free when it is not.
  for sw in $(awk 'NR>1{print $1}' /proc/swaps 2>/dev/null); do
    case "${sw}" in
      "${BREAK_RAID_DISK}"*) echo "  swapoff ${sw}"; swapoff "${sw}" || true
                             SW_UUID="$(blkid -s UUID -o value "${sw}" 2>/dev/null || true)"
                             [ -n "${SW_UUID}" ] && sed -i "\|UUID=${SW_UUID}|d" /etc/fstab ;;
    esac
  done

  for md in /dev/md*; do
    [ -b "${md}" ] || continue
    for part in $(mdadm --detail "${md}" 2>/dev/null \
                  | awk '/\/dev\/(sd|nvme|vd)/ {print $NF}'); do
      # Match by PARENT DISK, not a digit-suffix regex — `nvme0n1p2` is not
      # `<disk><digits>`, so the old pattern never matched NVMe at all.
      [ "/dev/$(lsblk -no PKNAME "${part}" 2>/dev/null | head -1)" = "${BREAK_RAID_DISK}" ] || continue
      echo "  ${md}: fail+remove ${part}"
      mdadm "${md}" --fail "${part}"   || true
      mdadm "${md}" --remove "${part}" || true
    done
  done

  # Drop the now-stale array members so mdadm does not re-add on reboot.
  mdadm --zero-superblock "${BREAK_RAID_DISK}"* 2>/dev/null || true
  wipefs -a "${BREAK_RAID_DISK}" 2>/dev/null || true

  # VERIFY the wipe rather than announcing it. The previous version printed
  # "is now free" unconditionally, which was a lie whenever the device was
  # still claimed — and the next step then walked into a corrupt fstab.
  if blkid -p "${BREAK_RAID_DISK}" >/dev/null 2>&1 || \
     lsblk -no TYPE "${BREAK_RAID_DISK}" | grep -q '^part$'; then
    echo "FATAL: ${BREAK_RAID_DISK} is still claimed after the reclaim attempt:" >&2
    lsblk "${BREAK_RAID_DISK}" >&2; blkid "${BREAK_RAID_DISK}"* 2>/dev/null >&2 || true
    echo "       Refusing to report it free. Investigate before using --data-disk." >&2
    exit 1
  fi

  echo "--- array state after reclaim:"
  cat /proc/mdstat || true
  echo "--- ${BREAK_RAID_DISK} verified free; pass it as --data-disk"
fi

# ---------------------------------------------------------------------------
log "VM data directory (reflink-capable)"
# ---------------------------------------------------------------------------
# P4 measures `cp --reflink=auto` of the rootfs per launch, and [D5]'s
# per-launch-copy design rests on the result. ext4 has NO FICLONE support, so
# on ext4 the measurement silently degrades to a full copy and you would
# wrongly conclude reflink does not help. XFS (reflink=1 is the mkfs default
# now) or btrfs is required for that number to mean anything.
#
# Formatting a disk is destructive and irreversible, so this NEVER auto-detects
# a device. Pass --data-disk explicitly or the step is skipped.
if [ -n "${DATA_DISK}" ]; then
  [ -b "${DATA_DISK}" ] || { echo "FATAL: ${DATA_DISK} is not a block device" >&2; exit 1; }

  # Reject a WHOLE DISK that has partitions. Passing /dev/sdb (rather than
  # /dev/sdb1) used to produce a silent disaster: `blkid /dev/sdb` exits 0 for
  # a bare partition TABLE, so the "already formatted" branch fired and mkfs
  # was skipped; then `blkid -s UUID` returned EMPTY, appending a malformed
  # fstab line with no device field, whose failed mount killed the script AND
  # persisted — so every later run died at the same place. Fail early instead.
  if [ "$(lsblk -no TYPE "${DATA_DISK}" | head -1)" = "disk" ] \
     && lsblk -no TYPE "${DATA_DISK}" | grep -q '^part$'; then
    echo "FATAL: ${DATA_DISK} is a whole disk with partitions." >&2
    echo "       Pass the PARTITION instead, e.g. ${DATA_DISK}1." >&2
    exit 1
  fi

  # Already converged? Re-running with the same args must be a no-op, not a
  # FATAL. The old mounted-check could not tell "mounted where I want it" from
  # "mounted somewhere unexpected", so the second run always died.
  if [ "$(findmnt -no TARGET -S "${DATA_DISK}" 2>/dev/null || true)" = "${VM_DATA_DIR}" ]; then
    echo "${DATA_DISK} already mounted at ${VM_DATA_DIR} ($(findmnt -no FSTYPE -T "${VM_DATA_DIR}")) — nothing to do"
  else
    if findmnt -S "${DATA_DISK}" >/dev/null 2>&1 || \
       lsblk -no MOUNTPOINT "${DATA_DISK}" | grep -q .; then
      echo "FATAL: ${DATA_DISK} (or a partition of it) is mounted elsewhere — refusing to format" >&2
      exit 1
    fi
    if blkid -p -u filesystem "${DATA_DISK}" >/dev/null 2>&1; then
      echo "NOTE: ${DATA_DISK} already carries a filesystem:"
      blkid "${DATA_DISK}"
      echo "      Re-formatting would destroy it. Skipping — wipe it by hand if intended."
    else
      echo "formatting ${DATA_DISK} as XFS with reflink=1"
      mkfs.xfs -m reflink=1 "${DATA_DISK}"
    fi

    DATA_UUID="$(blkid -s UUID -o value "${DATA_DISK}" || true)"
    if [ -z "${DATA_UUID}" ]; then
      echo "FATAL: ${DATA_DISK} has no filesystem UUID after mkfs — refusing to" >&2
      echo "       write an fstab entry with an empty device field." >&2
      exit 1
    fi
    mkdir -p "${VM_DATA_DIR}"
    # Anchored match: a bare substring grep is defeated by any comment that
    # merely mentions the path.
    grep -qE "[[:space:]]${VM_DATA_DIR}[[:space:]]" /etc/fstab || \
      echo "UUID=${DATA_UUID} ${VM_DATA_DIR} xfs defaults,noatime 0 2" >> /etc/fstab
    mountpoint -q "${VM_DATA_DIR}" || mount "${VM_DATA_DIR}"
  fi
  chown "${BUILD_USER}" "${VM_DATA_DIR}"
else
  echo "no --data-disk given; skipping storage setup."
  echo "P4's reflink measurement needs XFS(reflink=1) or btrfs — ext4 has no FICLONE."
  mkdir -p "${VM_DATA_DIR}"; chown "${BUILD_USER}" "${VM_DATA_DIR}"
fi

# Prove reflink actually works where the probes will run.
log "reflink capability at ${VM_DATA_DIR}"
_probe="${VM_DATA_DIR}/.reflink-probe"
dd if=/dev/zero of="${_probe}" bs=1M count=8 status=none
if cp --reflink=always "${_probe}" "${_probe}.clone" 2>/dev/null; then
  echo "+++ FICLONE supported on $(findmnt -no FSTYPE -T "${VM_DATA_DIR}") — P4 can measure the real thing"
else
  echo "!!! FICLONE NOT supported on $(findmnt -no FSTYPE -T "${VM_DATA_DIR}")"
  echo "!!! P4 would measure a full copy and misreport [D5]. Use --data-disk."
fi
rm -f "${_probe}" "${_probe}.clone"

log "provisioning complete — now run infra/provision/common-user.sh as ${BUILD_USER}"
