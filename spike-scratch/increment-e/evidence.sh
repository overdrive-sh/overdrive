#!/usr/bin/env bash
# PROBE increment-e — drive every P6 mode and collect the evidence in one pass.
#
# On env A this script's ancestor existed mostly to prove `shared=on` never
# booted. On env B it exists to answer the questions that failure blocked:
#   * does the volume round-trip work, in both directions
#   * is [D8g]'s host-side read_only actually enforced against a guest that
#     mounts the share READ-WRITE and tries to write
#   * what does a failed mount look like from inside (the refuse-to-exec errno)
#   * what does virtiofs cost, and does [D8c]'s `--cache=never` choice pay
#   * what does `shared=on` cost host-side, and does it change guest MemTotal
#   * does the RLIMIT_FSIZE x memfd interaction reproduce on x86_64
#
# Run as root on the bare-metal box. Output goes to $OUT/evidence.txt.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-e
EV="$OUT/evidence.txt"
mkdir -p "$OUT"

run_mode() {   # <label> <mode> [env assignments...]
  local label="$1" mode="$2"; shift 2
  echo
  echo "################################################################"
  echo "### $label"
  echo "###   mode=$mode  ${*:-}"
  echo "################################################################"
  env "$@" "$HERE/run.sh" "$mode" 2>&1
  echo "### $label rc=$?"
}

{
echo "=================================================================="
echo "PROBE increment-e — P6 evidence, bare metal"
echo "date        : $(date -Is)"
echo "host        : $(hostname)"
echo "kernel      : $(uname -r)  arch=$(uname -m)"
echo "virt        : $(systemd-detect-virt || true)"
echo "cpu         : $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
echo "ch          : $(cloud-hypervisor --version 2>&1)"
echo "virtiofsd   : $(/usr/libexec/virtiofsd --version 2>&1)"
echo "=================================================================="

# --- 1. the four memory/device modes, cache=never -------------------------
run_mode "A. full — shared=on + 2 fs devices + ALL P5 confinement" full
run_mode "B. full-no-fsd-rule — is a --fs socket= landlock rule auto-derived?" full-no-fsd-rule
run_mode "C. sharedonly — shared=on, NO fs devices" sharedonly
run_mode "D. noshare — the [D8b] volume-free baseline" noshare

# --- 2. [D8c]: is --cache=never the right default? ------------------------
# Same payload, same binary, same box; only the virtiofsd cache mode differs.
run_mode "E. full @ --cache=auto (the [D8c] comparison)" full CACHE=auto

# --- 3. [D8b]: does shared=on change what the guest sees? -----------------
echo
echo "################################################################"
echo "### F. [D8b] guest-visible MemTotal across memory backings"
echo "################################################################"
# The collector prefixes every line with '[HOST t=+N.NNNs] ', so anchoring the
# pattern at ^ matches nothing and the fallback prints a misleading
# '<no transcript>' when the transcript is in fact right there.
for m in full sharedonly noshare; do
  printf '  %-12s ' "$m"
  grep -ho 'MEMTOTAL.*' "$OUT/transcript-$m-never.txt" 2>/dev/null | head -1 \
    || echo "<transcript missing: $OUT/transcript-$m-never.txt>"
done
echo "  ^ identical across backings => shared=on does not change what the guest sees" 

# --- 4. host-side cost of shared=on, at the same guest lifecycle point ----
echo
echo "################################################################"
echo "### G. host-side VMM memory at the beacon (128 MiB touched, no fs I/O yet)"
echo "################################################################"
for m in full sharedonly noshare; do
  echo "--- mode=$m"
  grep -E '^(VmPeak|VmSize|VmHWM|VmRSS|RssAnon|RssFile|RssShmem|Threads):' "$OUT/mem-$m.txt" 2>/dev/null \
    | sed 's/^/    /' || echo "    <no capture>"
  grep -E 'memfd-ish mapping lines|/memfd:' "$OUT/mem-$m.txt" 2>/dev/null | sed 's/^/    /'
done

# --- 5. RLIMIT_FSIZE x memfd, re-measured on x86_64 ----------------------
# env A found the ceiling tracks `--memory size`, not the rootfs, because
# shared=on backs guest RAM with a memfd and a memfd is a FILE for RLIMIT_FSIZE.
# That was measured on aarch64; it is a mechanism claim, so it gets re-run here
# rather than carried over.
echo
echo "################################################################"
echo "### H. RLIMIT_FSIZE x memfd sweep (expect SIGXFSZ -> rc=153 below guest RAM)"
echo "################################################################"
"$HERE/rlimit-sweep.sh" 2>&1

echo
echo "=================================================================="
echo "END increment-e evidence  $(date -Is)"
echo "=================================================================="
} | tee "$EV"

echo
echo "evidence written to $EV ($(wc -l <"$EV") lines)"
