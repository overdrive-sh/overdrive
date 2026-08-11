#!/usr/bin/env bash
# PROBE increment-d — re-run the two cheap decisive experiments and tee RAW
# output. The two expensive ones (stalldiff, sharedon-attribution) are appended
# from their own captured run logs by assemble.sh; nothing is transcribed.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EV="$HERE/evidence-part2.txt"
{
  echo "############################################################################"
  echo "### PROBE increment-d — P6 evidence, part 2 (the cheap decisive experiments)"
  echo "### captured: $(date -Is)"
  echo "############################################################################"
  echo
  echo "############ ENVIRONMENT ############"
  echo "uname -r         : $(uname -r)"
  echo "uname -m         : $(uname -m)   <- DEV-HOST ARTIFACT; production/CI is x86_64"
  echo "cloud-hypervisor : $(cloud-hypervisor --version 2>&1)"
  echo "virtiofsd        : $(/usr/libexec/virtiofsd --version 2>&1)"
  echo "virtiofsd path   : /usr/libexec/virtiofsd"
  echo "virtiofsd on PATH: $(command -v virtiofsd || echo '<NOT ON PATH — command -v returns nothing>')"
  echo "qemu             : $(qemu-system-aarch64 --version 2>/dev/null | head -1)"
  echo "systemd-detect-virt: $(systemd-detect-virt 2>/dev/null)"
  echo "guest fs config  : $(grep -E 'CONFIG_(FUSE_FS|VIRTIO_FS)=' /boot/config-$(uname -r) | tr '\n' ' ')"
  echo
  echo "############ virtiofsd --sandbox and --readonly surface ([D8d], [D8g]) ############"
  /usr/libexec/virtiofsd --help 2>&1 | grep -A4 -E '^\s+--sandbox|^\s+--readonly|^\s+--cache|^\s+--socket-group'
  echo
  echo
  echo "############################################################################"
  echo "### EXPERIMENT D-1 — the P5 x P6 interaction: --memory shared=on backs guest"
  echo "### RAM with a memfd, and a memfd counts as a FILE for RLIMIT_FSIZE."
  echo "### (surfaced as CH exit 153 = 128+SIGXFSZ on every `full` attempt)"
  echo "############################################################################"
  "$HERE/rlimitdiff.sh"
  echo
  echo
  echo "############################################################################"
  echo "### EXPERIMENT D-4 — which Landlock rules does CH need once the fs device"
  echo "### is present, and does it ever need the volume SOURCE directory? ([D8e])"
  echo "###"
  echo "### Answerable despite the shared=on boot stall: CH builds its ruleset and"
  echo "### creates every device (incl. connecting to the vhost-user socket) BEFORE"
  echo "### the guest kernel runs, so a run that reaches the guest's virtio_blk"
  echo "### probe has already created its fs device."
  echo "############################################################################"
  "$HERE/fsdlandlock.sh"
} 2>&1 | tee "$EV"
