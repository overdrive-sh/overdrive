echo "== job2: loopback chain, inbound shapes, serial 10000 + x8 (8 lanes x 1250)"
uname -r
/pairprobe probe
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
i=0
for s in in-sf in-sffin in-cf in-cfearly; do
  i=$((i+1))
  /pairprobe L $s 1 10000 $((300+i)) 8000 $((300+i))
  /pairprobe L $s 8 1250 $((400+i)) 8000 $((400+i))
done
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
dmesg | grep -E 'ovdpair|BUG|WARN|Oops|Call Trace' | tail -20
