echo "== F1: loopback chain, egress shapes. serial 10000, x8 (8 lanes x 1250 = 10000), stress x8 (8 lanes x 10000 = 80000)"
uname -r
/pairprobe probe
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
i=0
for s in eg-wf eg-sf eg-sffin eg-nofin eg-rst eg-drst; do
  i=$((i+1))
  /pairprobe L $s 1 10000 $((1100+i)) 8000 $((1100+i))
  /pairprobe L $s 8 1250 $((1200+i)) 8000 $((1200+i))
done
j=0
for s in eg-wf eg-sf eg-sffin eg-drst; do
  j=$((j+1))
  /pairprobe L $s 8 10000 $((1300+j)) 8000 $((1300+j))
done
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
dmesg | grep -E 'ovdpair|BUG|WARN|Oops|Call Trace' | tail -20
