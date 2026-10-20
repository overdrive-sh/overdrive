echo "== job1: loopback chain, egress shapes, serial 10000 + x8 (8 lanes x 1250)"
uname -r
/pairprobe probe
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
i=0
for s in eg-wf eg-sf eg-sffin eg-nofin eg-rst; do
  i=$((i+1))
  /pairprobe L $s 1 10000 $((100+i)) 8000 $((100+i))
  /pairprobe L $s 8 1250 $((200+i)) 8000 $((200+i))
done
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
dmesg | grep -E 'ovdpair|BUG|WARN|Oops|Call Trace' | tail -20
