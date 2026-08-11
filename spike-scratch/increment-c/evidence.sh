#!/usr/bin/env bash
# PROBE increment-c — re-run every P5 experiment and tee the RAW output to
# evidence.txt. Nothing in that file is transcribed by hand; it is what the
# commands actually printed.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EV="$HERE/evidence.txt"

{
  echo "############################################################################"
  echo "### PROBE increment-c — P5 evidence (microvm-driver-cloud-hypervisor)"
  echo "### slice-00 § P5 — do the [D7] confinement flags compose with a real boot?"
  echo "### captured: $(date -Is)"
  echo "############################################################################"
  echo
  echo "############ ENVIRONMENT — every verdict below is pinned to this ############"
  echo "uname -r         : $(uname -r)"
  echo "uname -m         : $(uname -m)   <- DEV-HOST ARTIFACT; production/CI is x86_64"
  echo "uname -a         : $(uname -a)"
  echo "cloud-hypervisor : $(cloud-hypervisor --version 2>&1)"
  echo "active LSMs      : $(cat /sys/kernel/security/lsm)"
  echo "/dev/kvm (as found, Lima udev rule): $(stat -c '%A %U:%G' /dev/kvm)"
  echo "kvm group        : $(getent group kvm)"
  echo
  echo
  echo "############################################################################"
  echo "### EXPERIMENT 1 — population diff: WHICH confinement mechanism broke the"
  echo "### launch? (the first confined run died with an opaque"
  echo "###  CreateVsockBackend(UnixBind(EACCES)) that never mentions Landlock)"
  echo "############################################################################"
  "$HERE/matrix.sh"
  echo
  echo
  echo "############################################################################"
  echo "### EXPERIMENT 2 — which paths does CH v46's IMPLICIT Landlock ruleset"
  echo "### cover, and which does it miss? (the [D7] rule-list deliverable)"
  echo "############################################################################"
  "$HERE/matrix2.sh"
  echo
  echo
  echo "############################################################################"
  echo "### EXPERIMENT 3 — the P5 hypothesis itself: all four mechanisms at once,"
  echo "### on the same VM that booted in increment-a, PLUS the Landlock denial"
  echo "### that US-VM-7 AC 1(b) cites and that cannot be reconstructed later."
  echo "############################################################################"
  "$HERE/run.sh" confined 12
  echo
  echo
  echo "############################################################################"
  echo "### EXPERIMENT 4 — the two follow-ups experiment 3 raised:"
  echo "###   (a) /proc/<pid>/status said Seccomp: 0 despite --seccomp true"
  echo "###   (b) the guest did not reach 'reboot: Power down' and CH was SIGKILLed"
  echo "############################################################################"
  "$HERE/exitcheck.sh" 3
  echo
  echo "############################ END increment-c evidence ######################"
} 2>&1 | tee "$EV"
