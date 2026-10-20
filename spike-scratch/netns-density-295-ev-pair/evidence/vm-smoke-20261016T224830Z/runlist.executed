echo "== smoke"
uname -r
/pairprobe probe
cat /proc/ovd_fwd
n=1
for s in eg-wf eg-sf eg-sffin eg-nofin eg-rst in-sf in-sffin in-cf in-cfearly; do
  /pairprobe L $s 1 40 7 4000 $n
  n=$((n+1))
done
/pairprobe G eg-wf 1 40 7 4000 21 2 3371
/pairprobe G in-cfearly 1 40 7 4000 22 2 3371
cat /proc/ovd_fwd
dmesg | tail -30
