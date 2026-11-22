#!/usr/bin/env bash
# Probe: Cloud Hypervisor's exit status when a block device's exclusive (O_EXCL)
# open returns EBUSY. Run as root on the metal host:
#   cargo xtask metal run --exclusive -- spike-scratch/node-local-persistent-volume-ch-exit/probe.sh
# Prediction: held device -> exit 111; nonexistent disk -> exit 1; free device
# (control) -> opens, the VMM keeps running until `timeout` kills it (124).
set -u
CH=/usr/local/bin/cloud-hypervisor
KERNEL="${OVERDRIVE_METAL_KERNEL:?OVERDRIVE_METAL_KERNEL not set}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d /var/tmp/ch-exit-probe.XXXXXX)"
LOOP=""
HOLDER=""
cleanup() {
  [ -z "${HOLDER}" ] || kill "${HOLDER}" 2>/dev/null
  [ -z "${LOOP}" ] || losetup -d "${LOOP}" 2>/dev/null
  rm -rf "${WORK}"
}
trap cleanup EXIT

echo "uname -r: $(uname -r)"
echo "cloud-hypervisor sha256: $(sha256sum "${CH}" | cut -d' ' -f1)"
echo "kernel: ${KERNEL}"

truncate -s 64M "${WORK}/vol.img"
LOOP="$(losetup --find --show "${WORK}/vol.img")" || { echo "losetup failed"; exit 2; }
echo "volume device: ${LOOP}"

launch() { # $1 = disk path ; extra env TIMEOUT
  timeout "${TIMEOUT:-20}" "${CH}" --cpus boot=1 --memory size=128M \
    --kernel "${KERNEL}" --cmdline "console=ttyS0" \
    --disk "path=$1,image_type=raw,direct=on" \
    --guest-reset-action exit-vmm --api-socket "${WORK}/api.sock" 2>"${WORK}/stderr.txt"
  echo "exit status: $?"
  echo "--- stderr ---"; cat "${WORK}/stderr.txt"; echo "--------------"
  rm -f "${WORK}/api.sock"
}

echo; echo "== case 1: device held by an O_EXCL opener =="
python3 -I "${HERE}/hold.py" "${LOOP}" "${WORK}/held" &
HOLDER=$!
for _ in $(seq 50); do [ -e "${WORK}/held" ] && break; sleep 0.1; done
[ -e "${WORK}/held" ] || { echo "holder did not take the device"; exit 2; }
echo "holder pid ${HOLDER} holds ${LOOP}"
launch "${LOOP}"
kill "${HOLDER}"; wait "${HOLDER}" 2>/dev/null; HOLDER=""

echo; echo "== case 2: nonexistent disk =="
launch /dev/does-not-exist-ch-probe

echo; echo "== case 3 (control): device free =="
TIMEOUT=5 launch "${LOOP}"
