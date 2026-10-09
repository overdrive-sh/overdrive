#!/bin/sh
# Guest PID 1 for the V-11/VIP/V-14 spike (from the guest vsock capture spike). Stock kernel, stock modules.
# No network device except lo and a dummy carrying the workload address (no NIC).
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
mkdir -p /dev/pts /sys/fs/cgroup /sys/fs/bpf /tmp /run/sshd /var/empty
mount -t devpts devpts /dev/pts
mount -t cgroup2 none /sys/fs/cgroup
mount -t bpf bpf /sys/fs/bpf
mount -t tmpfs tmpfs /tmp
mount -t debugfs none /sys/kernel/debug 2>/dev/null
echo "GUEST_BOOT uname_r=$(uname -r) uptime=$(cut -d' ' -f1 /proc/uptime)"
for m in $(cat /mods/order); do insmod /mods/$m || echo "GUEST_INSMOD_FAIL $m"; done
W=$(sed -n 's/.*vfwd_w=\([^ ]*\).*/\1/p' /proc/cmdline)
IP=/usr/sbin/ip
$IP link set lo up
$IP link add dummy0 type dummy 2>/dev/null || true
$IP addr add "$W/32" dev dummy0
$IP link set dummy0 up
# Inbound transparency (D15): replies to a real client address are delivered
# locally for sockets/children carrying the owner's mark.
sysctl -w net.ipv4.tcp_fwmark_accept=1
# The guest's own RST to a refused inbound connect (no listener) must carry the
# acceptor's mark so the fwmark route delivers it to the transparent client socket.
sysctl -w net.ipv4.fwmark_reflect=1
$IP rule add fwmark 0x295 lookup 295
$IP route add local 0.0.0.0/0 dev lo table 295
echo "GUEST_NET_BEGIN"; $IP -br link; $IP -4 addr; $IP route; $IP rule; echo "GUEST_NET_END"
ls /sys/class/net
/vfwd agent > /tmp/agent.out 2>&1 &
OWNER_ARGS=$(sed -n 's/.*vfwd_owner=\([^ ]*\).*/\1/p' /proc/cmdline | tr ',' ' ')
echo "$OWNER_ARGS" > /tmp/owner.args
RUST_BACKTRACE=1 /vfwd guest $OWNER_ARGS > /tmp/owner.out 2>&1 &
echo $! > /tmp/owner.pid
/vfwd tcp-server --bind 0.0.0.0:7100 > /tmp/srv7100.log 2>&1 &
/vfwd tcp-server --bind 0.0.0.0:7101 --banner > /tmp/srv7101.log 2>&1 &
/usr/sbin/sshd -D -e -f /etc/ssh/sshd_config > /tmp/sshd.log 2>&1 &
sleep 1
cat /tmp/owner.out
echo "GUEST_READY"
while true; do sleep 3600; done
