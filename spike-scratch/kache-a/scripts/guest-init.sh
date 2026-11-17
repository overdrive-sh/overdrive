#!/bin/bash
# PID 1 inside a spike guest (exec'd by spike-init after it mounts the host
# root read-only over virtiofs). Mounts scratch filesystems, the guest's own
# data disk at /mnt and the shared bus at /media, configures the NIC, runs
# scripts/role-$SPIKE_ROLE.sh, records its exit code and powers off.
set -u
mount -t proc proc /proc
ln -sfn /proc/self/fd /dev/fd; ln -sfn /proc/self/fd/0 /dev/stdin; ln -sfn /proc/self/fd/1 /dev/stdout; ln -sfn /proc/self/fd/2 /dev/stderr
mount -t sysfs sys /sys
for d in /run /tmp /var/tmp /root; do mount -t tmpfs tmpfs "$d"; done
mount -t ext4 /dev/vda /mnt
mount -t virtiofs bus /media
mkdir -p "/media/$SPIKE_NAME"
exec > >(tee -a "/media/$SPIKE_NAME/guest.log") 2>&1

ip link set lo up
nic="$(ls /sys/class/net | grep -v '^lo$' | head -1)"
ip addr add "$SPIKE_IP/24" dev "$nic"
ip link set "$nic" up

export HOME=/mnt/home
export PATH=/home/ubuntu/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export RUSTUP_HOME=/home/ubuntu/.rustup
export CARGO_HOME=/mnt/cargo-home
export BUS=/media
export ME="/media/$SPIKE_NAME"
mkdir -p "$HOME" "$CARGO_HOME"

glog() { printf '%s %s %s\n' "$(date -u +%FT%T.%3NZ)" "$SPIKE_NAME" "$*"; }
glog "up nic=$nic ip=$SPIKE_IP role=$SPIKE_ROLE kernel=$(uname -r) nproc=$(nproc)"

# shellcheck source=/dev/null
( source "$SPIKE_DIR/scripts/lib.sh"; source "$SPIKE_DIR/scripts/role-$SPIKE_ROLE.sh" )
rc=$?
glog "role exit=$rc"
echo "$rc" > "$ME/exit"
sync
umount /mnt 2>/dev/null || sync
echo o > /proc/sysrq-trigger
sleep 30
