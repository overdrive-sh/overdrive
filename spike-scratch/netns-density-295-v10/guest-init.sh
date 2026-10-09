#!/bin/sh
# V-10 guest PID 1 (THROWAWAY SPIKE). No NIC. Loads the stock vsock modules,
# optionally arms the forge module, runs the vsock client against the host,
# prints everything to the serial console, then powers off.
#
# Kernel cmdline carries v10_* key=val pairs:
#   v10_mode=plain|midstream  v10_guestcid=<N>  v10_forge=<0|N>
#   v10_sport=<N> v10_qport=<N>
set +e
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sysfs /sys 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null
echo "===== V10 guest boot ====="
uname -a

mode=plain; guestcid=0; forge=0; sport=1234; qport=1235
for kv in $(cat /proc/cmdline); do
  case "$kv" in
    v10_*) eval "${kv#v10_}" 2>/dev/null ;;
  esac
done
echo "V10 params: mode=$mode guestcid=$guestcid forge=$forge sport=$sport qport=$qport"

echo "--- insmod stock vsock modules ---"
while read m; do
  [ -z "$m" ] && continue
  if insmod "/mods/$m"; then echo "insmod $m OK"; else echo "insmod $m rc=$?"; fi
done < /mods/order

echo "--- symbol present? ---"
echo "virtio_transport_get_local_cid in kallsyms: $(grep -c virtio_transport_get_local_cid /proc/kallsyms)"

if [ "$forge" != "0" ] && [ "$mode" = "plain" ]; then
  echo "--- arming forge (plain) forge_cid=$forge ---"
  if insmod /forge.ko forge_cid=$forge; then echo "forge insmod OK"; else echo "forge insmod rc=$?"; fi
elif [ "$mode" = "midstream" ]; then
  echo "--- loading forge DISARMED (midstream; client flips it) ---"
  if insmod /forge.ko forge_cid=0; then echo "forge insmod OK (disarmed)"; else echo "forge insmod rc=$?"; fi
else
  echo "--- control mode: no forge module ---"
fi
echo "forge param now: $(cat /sys/module/vsock_forge/parameters/forge_cid 2>/dev/null || echo n/a)"

sleep 1
echo "--- run client ---"
if [ "$mode" = "midstream" ]; then
  /client midstream "$guestcid" "$sport" "$forge"
else
  /client plain "$guestcid" "$sport" "$qport"
fi
echo "client exit rc=$?"

sleep 1
echo "--- guest dmesg (forge) ---"
dmesg | grep -i vsock_forge
echo "===== V10 guest done ====="
sync
poweroff -f
