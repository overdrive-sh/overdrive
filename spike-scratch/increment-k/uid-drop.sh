#!/usr/bin/env bash
# PROBE increment-k — does `memory_restore_mode=OnDemand` survive P5's
# uid-dropped launch shape?
#
# This is a COMPOSITION question, and it is the one that decides whether the
# feature is usable at all in the shape `[D7]` / US-VM-7 already committed to.
#
#   * P5 settled that production CH runs uid-dropped:
#       prlimit ... setpriv --reuid=spikevmm --regid=6001 --init-groups
#               --no-new-privs -- cloud-hypervisor ... --seccomp true --landlock
#   * `OnDemand` restore is implemented with userfaultfd (the VMM grows a
#     thread literally named `uffd-handler`, see the bench evidence).
#   * This box has `/proc/sys/vm/unprivileged_userfaultfd = 0`, the distro
#     default, which restricts userfaultfd(2) to processes holding
#     CAP_SYS_PTRACE.
#
# So the two established facts point opposite ways and only a run settles it.
# Both modes are attempted under the SAME dropped uid, so a failure that is
# really about file permissions shows up in BOTH arms and cannot be
# misattributed to userfaultfd.
set -uo pipefail

OUT=/var/tmp/spike-increment-k
JAIL=/srv/vm/p13k-uid
VMMUSER=spikevmm
SRC_SNAP="${1:-$(ls -d /srv/vm/p13k/snap-*2048 2>/dev/null | head -1)}"
[ -d "$SRC_SNAP" ] || { echo "!!! no snapshot; run bench.sh first" >&2; exit 1; }

PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill -9 "$p" 2>/dev/null; done; }
trap cleanup EXIT

echo "##################################################################"
echo "### increment-k — OnDemand restore under P5's uid-dropped shape"
echo "### unprivileged_userfaultfd = $(cat /proc/sys/vm/unprivileged_userfaultfd)"
echo "### source snapshot          = $SRC_SNAP"
echo "##################################################################"

id "$VMMUSER" >/dev/null 2>&1 || useradd -M -s /usr/sbin/nologin -G kvm "$VMMUSER"
UID_N="$(id -u "$VMMUSER")"; GID_N="$(id -g "$VMMUSER")"
echo "### $VMMUSER uid=$UID_N gid=$GID_N groups=$(id -Gn "$VMMUSER")"
echo "### /dev/kvm: $(stat -c '%A %U:%G' /dev/kvm)"

rm -rf "$JAIL"; mkdir -p "$JAIL"
cp -a --reflink=auto "$SRC_SNAP" "$JAIL/snap"
cp --reflink=auto "$OUT/rootfs.ext4" "$JAIL/rootfs.ext4"
python3 - "$JAIL/snap/config.json" "$JAIL/rootfs.ext4" "$JAIL/console.log" <<'PY'
import json,sys
p,disk,ser=sys.argv[1],sys.argv[2],sys.argv[3]
c=json.load(open(p))
for d in (c.get("disks") or []): d["path"]=disk
if c.get("serial"): c["serial"]["file"]=ser
json.dump(c,open(p,"w"))
PY
: >"$JAIL/console.log"
chown -R "$UID_N:$GID_N" "$JAIL"
chmod -R u+rwX "$JAIL"

for MODE in Copy OnDemand; do
  echo
  echo "=================================================================="
  echo "=== mode=$MODE  as uid=$UID_N (setpriv --no-new-privs), NOT root"
  echo "=================================================================="
  S="$JAIL/api-$MODE.sock"
  L="$JAIL/ch-$MODE.log"
  rm -f "$S" "$S.lock"
  prlimit --nofile=256 -- \
    setpriv --reuid="$VMMUSER" --regid="$GID_N" --init-groups --no-new-privs -- \
    cloud-hypervisor --api-socket "path=$S" --seccomp true >"$L" 2>&1 &
  PIDS+=($!)
  P="${PIDS[-1]}"
  for _ in $(seq 1 200); do [ -S "$S" ] && break; sleep 0.05; done
  if [ ! -S "$S" ]; then
    echo "  !!! API socket never appeared; VMM log:"; sed 's/^/      /' "$L"; continue
  fi
  echo "  VMM pid=$P  running as uid=$(awk '/^Uid:/{print $2}' /proc/$P/status)"
  T0=$(date +%s.%N)
  C="$(curl -s -o "$JAIL/resp-$MODE.txt" -w '%{http_code}' --unix-socket "$S" -X PUT \
        -H 'Content-Type: application/json' \
        -d "{\"source_url\":\"file://$JAIL/snap\",\"memory_restore_mode\":\"$MODE\"}" \
        http://localhost/api/v1/vm.restore)"
  T1=$(date +%s.%N)
  echo "  PUT vm.restore -> HTTP $C   in $(echo "$T1 - $T0" | bc)s"
  [ -s "$JAIL/resp-$MODE.txt" ] && sed 's/^/      body: /' "$JAIL/resp-$MODE.txt" && echo
  R="$(curl -s -o /dev/null -w '%{http_code}' --unix-socket "$S" -X PUT \
        http://localhost/api/v1/vm.resume)"
  echo "  PUT vm.resume  -> HTTP $R"
  sleep 3
  if kill -0 "$P" 2>/dev/null; then
    echo "  VmRSS=$(awk '/^VmRSS:/{print $2}' /proc/$P/status) kB   threads:"
    for t in /proc/$P/task/*/comm; do echo "      $(cat "$t")"; done | sort | uniq -c | sed 's/^/    /'
    echo "  guest ticks observed: $(grep -c '^TICK' "$JAIL/console.log" 2>/dev/null || echo 0)"
  else
    echo "  !!! VMM is GONE"
  fi
  echo "  --- VMM log (errors):"
  grep -iE "error|fail|perm|userfault|uffd" "$L" | head -5 | sed 's/^/      /'
  kill -9 "$P" 2>/dev/null; PIDS=(); sleep 0.5
  : >"$JAIL/console.log"
done

rm -rf "$JAIL"
echo
echo "=== uid-drop probe DONE"
