echo "== F2: loopback chain, inbound shapes. serial 10000, x8 (8 lanes x 1250), stress x8 (8 lanes x 10000)"
uname -r
/pairprobe probe
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
i=0
for s in in-sf in-sffin in-cf in-cfearly; do
  i=$((i+1))
  /pairprobe L $s 1 10000 $((1400+i)) 8000 $((1400+i))
  /pairprobe L $s 8 1250 $((1500+i)) 8000 $((1500+i))
done
j=0
for s in in-sffin in-cfearly in-cf; do
  j=$((j+1))
  /pairprobe L $s 8 10000 $((1600+j)) 8000 $((1600+j))
done
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
dmesg | grep -E 'ovdpair|BUG|WARN|Oops|Call Trace' | tail -20
