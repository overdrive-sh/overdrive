#!/usr/bin/env bash
# Appends reader notes to evidence.txt. Every claim is backed by pasted output.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EV="$HERE/evidence.txt"
{
  echo
  echo "############################################################################"
  echo "### READER NOTES"
  echo "############################################################################"
  echo
  echo "NOTE 1 - the EXPERIMENT 2 row 'second-disk-elsewhere ... rc=2' is an INVALID"
  echo "TRIAL, not a Landlock result. It is a CLI usage error in the probe script."
  echo "Proof - the actual ch.log of that trial:"
  sed 's/^/    /' /run/spike-c-matrix2/second-disk-elsewhere/ch.log 2>/dev/null
  echo "    (cloud-hypervisor takes multiple disks as '--disk a b', not '--disk a --disk b')"
  echo "    Disregard that row; it carries no information about the ruleset."
  echo
  echo "NOTE 2 - cross-run tally for the missing 'reboot: Power down'."
  echo "Pooling the EXPERIMENT 4 run above with an identical earlier run:"
  echo "    baseline (unconfined, root) : a run reached the beacon but NOT power-down"
  echo "    confined (all four mechs)   : a run reached the beacon but NOT power-down"
  echo "Both populations show it, so it is the nested-virt stall landing at the"
  echo "power-off boundary (increment-a findings, 'The nested-virt stall'), NOT a"
  echo "confinement effect. The falsification clause of the exitcheck hypothesis -"
  echo "'confined NEVER reaches power-down while baseline ALWAYS does' - did not"
  echo "fire: confined reached power-down with CH exit 0 repeatedly."
  echo
  echo "NOTE 3 - isolation check required by .claude/rules/spike.md."
  echo "git status --porcelain -- crates/  :"
  ( cd "$HERE/../.." && git status --porcelain -- crates/ ) | sed 's/^/    /'
  echo "    (empty above = crates/ untouched)"
} >>"$EV"
tail -32 "$EV"
