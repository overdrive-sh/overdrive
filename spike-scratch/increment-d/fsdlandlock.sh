#!/usr/bin/env bash
# PROBE increment-d — the CH-ruleset question, which IS answerable on this host
# even though the guest cannot boot under shared=on.
#
# Why it is still answerable: CH builds its Landlock ruleset and creates every
# device (including connecting to the virtiofsd vhost-user socket) BEFORE the
# guest kernel runs. The nested-virt MAP_SHARED stall happens later, at the
# guest's root mount. So "did CH's ruleset let it reach the vhost-user socket
# and the fs device" is decided strictly before the point where this host gives
# up, and a run that reaches the guest's `virtio_blk` probe has necessarily
# already created all its devices.
#
# Two questions:
#   Q1 does CH auto-derive a Landlock rule for `--fs socket=`, the way it does
#      for --kernel/--disk/--serial/--api-socket but NOT for --vsock?
#   Q2 does CH need the volume SOURCE directory in its ruleset? ([D8e] claims
#      only virtiofsd touches the data. If CH needs it, volumes widen [D7]'s
#      hypervisor confinement and US-VM-8's non-widening AC must be restated.)
#
# Hypothesis:        Q1 no (an explicit rule is needed, as for --vsock);
#                    Q2 no (CH never opens the source dir).
# Predicted outcome: without the socket-dir rule, CH fails at fs-device creation
#                    with a permission error naming the vhost-user socket; with
#                    it, CH reaches the guest kernel while the source dirs were
#                    never granted.
# Falsification:     it works without the rule (=> auto-derived), or it fails
#                    even WITH the socket rule until the source dir is granted
#                    (=> volumes widen the hypervisor's ruleset).
set -uo pipefail

A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-d
KERNEL="$A_OUT/Image"
ROOTFS="$A_OUT/rootfs.ext4"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"
VIRTIOFSD=/usr/libexec/virtiofsd
VMM_USER=spikevmm
VMM_GID=6001
VOLRW="$OUT/volsrc-rw"

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
cleanup() {
  pkill -9 -f "virtiofsd --socket-path=/run/spike-d-fsdll" 2>/dev/null
  chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true
}
trap cleanup EXIT
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm

mkdir -p "$VOLRW"; printf 'HOST-WROTE-THIS-9876543210-zyxwvutsrq\n' >"$VOLRW/from-host.txt"
chmod -R 0777 "$VOLRW"

echo "### uname -r: $(uname -r)  CH: $(cloud-hypervisor --version)  virtiofsd: $($VIRTIOFSD --version)"
echo "### volume SOURCE dir (never granted to CH in any trial below): $VOLRW"
echo

trial() {
  local name="$1"; shift
  local rules=("$@")
  local run="/run/spike-d-fsdll/$name"
  # SIBLING of $run, not a child. A child directory is transitively covered by
  # `path=$run,access=rw` (Landlock rules are path-BENEATH), so nesting it would
  # make the "no explicit rule" trial vacuous — it would be granted anyway.
  local fsd="/run/spike-d-fsdll/$name-FSD"
  pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null
  pkill -9 -f "virtiofsd --socket-path=/run/spike-d-fsdll" 2>/dev/null
  sleep 0.3
  rm -rf "$run" "$fsd"; mkdir -p "$run" "$fsd"; cp "$ROOTFS" "$run/rootfs.ext4"

  $VIRTIOFSD --socket-path="$fsd/volrw.sock" --shared-dir="$VOLRW" --tag=volrw \
    --cache=never --sandbox=namespace --socket-group="$VMM_USER" \
    --log-level=info >"$fsd/fsd.log" 2>&1 &
  for _ in $(seq 1 60); do [ -S "$fsd/volrw.sock" ] && break; sleep 0.1; done
  chown -R 6001:6001 "$run"; chmod 0700 "$run"; chmod 0755 "$fsd"

  local expanded=()
  for r in "${rules[@]}"; do expanded+=("${r//__RUN__/$run}"); expanded[-1]="${expanded[-1]//__FSD__/$fsd}"; done

  timeout 45 \
    prlimit --fsize=$((1024*1024*1024)) --nofile=256 -- \
    setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs -- \
    cloud-hypervisor --cpus boot=1 --memory "size=512M,shared=on" \
      --kernel "$KERNEL" --cmdline "$CMDLINE" --disk "path=$run/rootfs.ext4" \
      --serial "file=$run/console.log" --console off \
      --api-socket "path=$run/ch-api.sock" \
      --fs "tag=volrw,socket=$fsd/volrw.sock" \
      --seccomp true --landlock --landlock-rules "${expanded[@]}" \
      >"$run/ch.log" 2>&1
  local rc=$?

  local err reached
  err="$(grep -m1 -E '^(Error|error)' "$run/ch.log" 2>/dev/null | head -c 190)"
  if grep -q 'virtio_blk' "$run/console.log" 2>/dev/null; then
    reached="GUEST KERNEL RAN (all devices created, incl. the fs device)"
  elif [ -s "$run/console.log" ]; then
    reached="guest kernel started"
  else
    reached="no guest output"
  fi
  echo "  --- $name"
  echo "      rules  : ${expanded[*]}"
  echo "      rc=$rc  $reached"
  [ -n "$err" ] && echo "      CH err : $err"
  echo "      fsd log: $(tr '\n' ' ' <"$fsd/fsd.log" 2>/dev/null | head -c 190)"
  pkill -9 -f "virtiofsd --socket-path=$fsd" 2>/dev/null
}

echo "=== Q1: is a --landlock-rules entry for the vhost-user socket dir required?"
trial no-fsd-rule    "path=__RUN__,access=rw"
echo
trial with-fsd-rule  "path=__RUN__,access=rw" "path=__FSD__,access=rw"
echo
echo "=== Q2: was the volume SOURCE dir ever granted? (it was not, in either trial)"
echo "    $VOLRW appears in no rule above. If 'with-fsd-rule' reached the guest"
echo "    kernel, CH created the fs device without ever being granted the source."
echo
echo "=== who actually opens the volume source? (virtiofsd's fds vs CH's)"
run=/run/spike-d-fsdll/whoopens; fsd=/run/spike-d-fsdll/whoopens-FSD
rm -rf "$run"; mkdir -p "$run" "$fsd"
$VIRTIOFSD --socket-path="$fsd/volrw.sock" --shared-dir="$VOLRW" --tag=volrw \
  --cache=never --sandbox=namespace --socket-group="$VMM_USER" \
  --log-level=info >"$fsd/fsd.log" 2>&1 &
fsdpid=$!
sleep 2
echo "--- virtiofsd pid=$fsdpid  cwd/root/fds:"
ls -l "/proc/$fsdpid/cwd" "/proc/$fsdpid/root" 2>/dev/null | sed 's/^/    /'
ls -l "/proc/$fsdpid/fd" 2>/dev/null | sed 's/^/    /' | head -12
echo "--- virtiofsd namespaces (does --sandbox=namespace actually unshare?):"
for ns in mnt pid user net; do
  printf '    %-5s virtiofsd=%s  shell=%s\n' "$ns" \
    "$(readlink /proc/$fsdpid/ns/$ns 2>/dev/null)" "$(readlink /proc/self/ns/$ns)"
done
kill -9 $fsdpid 2>/dev/null
exit 0
