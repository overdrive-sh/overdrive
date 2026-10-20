echo "== job3: real vhost-vsock, guest pairs against the host userspace peer, serial 10000 (+ x8 for two shapes)"
uname -r
/pairprobe probe
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
i=0
for s in eg-wf eg-sf eg-sffin eg-nofin in-sf in-sffin in-cf in-cfearly; do
  i=$((i+1))
  /pairprobe G $s 1 10000 $((500+i)) 8000 $((500+i)) 2 3371
done
/pairprobe G eg-rst 1 2000 509 8000 509 2 3371
/pairprobe G eg-wf 8 1250 601 8000 601 2 3371
/pairprobe G in-cfearly 8 1250 602 8000 602 2 3371
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
dmesg | grep -E 'ovdpair|BUG|WARN|Oops|Call Trace' | tail -20
