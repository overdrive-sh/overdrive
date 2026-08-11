#!/usr/bin/env bash
# PROBE increment-i / P11 — the four-arm interleaved benchmark.
#
# Closes the one thing P10 explicitly recorded as NOT established: what
# `vhost-user-blk` costs.
#
# THE ARMS. Same payload, same box, same XFS(reflink=1) filesystem, same guest
# instrument, interleaved so host-side drift cannot load onto one arm:
#
#   plain         ordinary --disk           , --memory size=512M
#   plain-shared  ordinary --disk           , --memory size=512M,shared=on
#   vublk         vhost-user-blk backend    , --memory size=512M,shared=on
#   virtiofs      virtiofsd --cache=never   , --memory size=512M,shared=on
#
# `plain-shared` exists ONLY to break the confound: `vhost-user-blk` is refused
# without `shared=on` (P10), plain `--disk` does not need it, so a bare
# plain-vs-vublk delta would also be a memory-backing delta. With both plain
# variants measured, the reader can see for themselves whether the backing
# moves the number before attributing anything to the transport.
#
# The `virtiofs` arm is increment-e's `run.sh full` — the established instrument
# for that mechanism, invoked verbatim rather than reimplemented, exactly as
# increment-f's `vs-virtiofs.sh` did. Its guest binary's payload functions are
# byte-identical to the block guest's (verified by diff), so the comparison is
# like-for-like at the syscall level and lands directly against P7's numbers.
#
# METHOD, and these are not negotiable — an earlier 3-trial run in this spike
# produced two outliers taken while the box was still settling after
# re-provisioning, and they would have reversed a conclusion:
#   * >= 5 interleaved trials per arm
#   * EVERY sample printed; nothing averaged away
#   * ranges reported, not means; overlap stated explicitly
#   * any discarded trial named, with the reason
#
# Usage: bench.sh [trials]      (default 5)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
E="$(cd "$HERE/../increment-e" && pwd)"
OUT=/var/tmp/spike-increment-i
RESULTS="$OUT/bench.txt"
TRIALS="${1:-5}"
mkdir -p "$OUT"
: >"$RESULTS"

ARMS=(plain plain-shared vublk virtiofs)

echo "=================================================================="
echo "P11 — vhost-user-blk cost: $TRIALS interleaved trials x ${#ARMS[@]} arms"
echo "host    : $(uname -r) $(uname -m)  virt=$(systemd-detect-virt || true)"
echo "cpu     : $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
echo "CH      : $(cloud-hypervisor --version 2>&1)"
echo "qsd     : $(qemu-storage-daemon --version 2>&1 | head -1)"
echo "virtiofsd: $(/usr/libexec/virtiofsd --version 2>&1 | head -1)"
echo "storage : $(findmnt -no FSTYPE,SOURCE --target /srv/vm)"
echo "payload : 256 MiB streaming write (1 MiB chunks + one fsync) + 1000 files"
echo "=================================================================="
echo

run_arm() {
  local arm="$1" i="$2"
  local log="$OUT/b-$arm-$i.log"
  printf 'trial %s %-13s ... ' "$i" "$arm"
  case "$arm" in
    virtiofs) VOLROOT=/srv/vm/p6 "$E/run.sh" full >"$log" 2>&1 ;;
    *)        "$HERE/run.sh" "$arm"               >"$log" 2>&1 ;;
  esac
  local rc=$?
  if [ $rc = 0 ]; then
    echo ok
    { echo "trial=$i arm=$arm"
      grep -ho 'FS-THROUGHPUT.*' "$log" | head -1 | sed 's/^/    /'
      grep -ho 'FS-LATENCY.*'    "$log" | head -1 | sed 's/^/    /'
      grep -h  'host 256 MiB'    "$log" | head -1
      grep -h  'host 1000 small' "$log" | head -1
      [ "$arm" = vublk ] && grep -h 'backend still alive' "$log" | head -1 | sed 's/^/    /'
      :; } >>"$RESULTS"
  else
    echo "INCOMPLETE (rc=$rc)"
    echo "trial=$i arm=$arm INCOMPLETE rc=$rc" >>"$RESULTS"
  fi
}

for i in $(seq 1 "$TRIALS"); do
  for arm in "${ARMS[@]}"; do run_arm "$arm" "$i"; done
done

echo
echo "=================================================================="
echo "ALL SAMPLES — every trial, nothing dropped"
echo "=================================================================="
cat "$RESULTS"

echo
python3 - "$RESULTS" "${ARMS[@]}" <<'PY'
import re, sys
path, arms = sys.argv[1], sys.argv[2:]
lines = open(path).read().split('\n')

# HARNESS CORRECTION, caught during the increment-i smoke run and worth stating
# plainly because P7 reports the number this replaces.
#
# The guest times write() and fsync() SEPARATELY, and the arms park their cost
# in different places: the plain smoke was write=0.079s fsync=0.182s, the vublk
# smoke was write=0.378s fsync=0.007s. Quoting "MiB/s (write only)" therefore
# compares how much each transport DEFERS, not how fast it is — the plain arm's
# write-only figure is high precisely because the bytes are still dirty in the
# guest page cache when the timer stops.
#
# The durable number is total = write + fsync. It is the headline here. The
# write-only figure is still printed, because it is what P7 quotes and dropping
# it silently would break the comparison this probe exists to make.
tput_total, tput_write, lat = ({a: [] for a in arms} for _ in range(3))
split = {a: [] for a in arms}
cur = None
for line in lines:
    m = re.search(r'arm=(\S+)$', line)
    if m and 'INCOMPLETE' not in line:
        cur = m.group(1); continue
    if cur is None:
        continue
    if 'FS-THROUGHPUT' in line:
        w = re.search(r'write=([0-9.]+)s fsync=([0-9.]+)s total=([0-9.]+)s', line)
        a = re.search(r'([0-9.]+) MiB/s \(write only\)', line)
        b = re.search(r'([0-9.]+) MiB/s \(incl\. fsync\)', line)
        if w: split[cur].append((float(w.group(1)), float(w.group(2))))
        if a: tput_write[cur].append(float(a.group(1)))
        if b: tput_total[cur].append(float(b.group(1)))
    if 'FS-LATENCY' in line:
        m = re.search(r'mean ([0-9.]+) ms/file', line)
        if m: lat[cur].append(float(m.group(1)))

def table(title, d, unit, fmt="{:.1f}"):
    print(f"\n=== {title}")
    for a in arms:
        v = d[a]
        print(f"    {a:<13} " + (" ".join(fmt.format(x) for x in v) if v else "<no samples>"))
    print(f"    {'':<13} (unit: {unit}; every trial shown, nothing averaged away)")

table("256 MiB DURABLE write, MiB/s incl. fsync  <-- the headline", tput_total, "MiB/s")
table("256 MiB write-only, MiB/s  (what P7 quotes; deferral-sensitive)", tput_write, "MiB/s")
table("1000 files, mean ms/file", lat, "ms/file", "{:.2f}")

print("\n=== write/fsync split, seconds (why write-only is not comparable)")
for a in arms:
    print(f"    {a:<13} " + " ".join(f"{w:.3f}/{f:.3f}" for w, f in split[a]))

def ranges(title, d, unit, hi_is_good):
    print(f"\n=== RANGES — {title}")
    for a in arms:
        v = d[a]
        if not v:
            print(f"    {a:<13} <no samples>"); continue
        print(f"    {a:<13} n={len(v)}  {min(v):.2f} .. {max(v):.2f} {unit}")
    print("    do the ranges overlap?")
    for i, a in enumerate(arms):
        for b in arms[i+1:]:
            if not d[a] or not d[b]:
                continue
            ov = not (max(d[a]) < min(d[b]) or max(d[b]) < min(d[a]))
            verdict = "OVERLAP -- no difference claimable" if ov else "DISJOINT -- difference is real"
            print(f"      {a:<13} vs {b:<13} {verdict}")

ranges("256 MiB durable write (higher is better)", tput_total, "MiB/s", True)
ranges("1000 files (lower is better)", lat, "ms/file", False)

print("\n=== THE VHOST-USER TRANSPORT DELTA")
print("    plain and vublk write THE SAME IMAGE FILE on THE SAME FILESYSTEM,")
print("    so this delta is the vhost-user transport + qemu's block layer.")
print("    It is NOT a projection of what overdrive-fs (#97) would cost.")
for base in ("plain", "plain-shared"):
    for label, d, better in (("durable MiB/s", tput_total, "hi"),
                             ("per-file ms  ", lat, "lo")):
        b, v = d.get(base), d.get("vublk")
        if not b or not v:
            continue
        mb, mv = sum(b)/len(b), sum(v)/len(v)
        rel = (mv/mb) if better == "hi" else (mv/mb)
        print(f"    {label}  vublk vs {base:<13}: {mv:.2f} vs {mb:.2f}  -> {rel:.2f}x")
PY
echo
echo "=================================================================="
echo "HOST BASELINE on the same XFS (matched syscall sequence)"
echo "=================================================================="
printf '    256 MiB write+fsync, s : '
grep -o 'host 256 MiB direct write+fsync: [0-9.]*' "$RESULTS" | sed 's/.*: //' | tr '\n' ' '; echo
printf '    1000 files, ms/file    : '
grep -o 'host 1000 small files.*mean [0-9.]* ms/file' "$RESULTS" | grep -o 'mean [0-9.]*' | sed 's/mean //' | tr '\n' ' '; echo
echo
echo "results: $RESULTS"
