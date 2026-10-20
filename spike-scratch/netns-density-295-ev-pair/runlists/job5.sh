echo "== job5: in-cfearly (the probe-defect rerun) then the hazard variants"
uname -r
/pairprobe probe
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
/pairprobe L in-cfearly 1 10000 304 8000 304
/pairprobe L in-cfearly 8 1250 404 8000 404
cat /proc/ovd_fwd
echo "-- control (all rules on), 500 trials"
/pairprobe L eg-wf 1 500 701 1500 701
/pairprobe L eg-sffin 1 500 702 1500 702
/pairprobe L in-cfearly 1 500 703 1500 703
echo "-- h_strict=1: attach requires TCP_ESTABLISHED (the evmod spike check)"
echo 1 > $P/h_strict
/pairprobe L eg-wf 1 500 711 1500 711
/pairprobe L in-sffin 1 500 712 1500 712
echo 0 > $P/h_strict
echo "-- h_noseed=1: no attach-time classification of a vsock end already received"
echo 1 > $P/h_noseed
/pairprobe L eg-sffin 1 500 721 1500 721
/pairprobe L in-cfearly 1 500 722 1500 722
/pairprobe L eg-wf 1 500 723 1500 723
echo 0 > $P/h_noseed
echo "-- h_fault=1: one byte dropped by the forwarder (checks the probe can see loss)"
echo 1 > $P/h_fault
/pairprobe L eg-wf 1 200 731 1500 731
/pairprobe L in-cf 1 200 732 1500 732
echo 0 > $P/h_fault
echo "-- h_nokick=1: no attach-time run (small, each stranded trial costs seconds)"
echo 1 > $P/h_nokick
/pairprobe L eg-wf 1 20 741 600 741
/pairprobe L in-cfearly 1 20 742 600 742
echo 0 > $P/h_nokick
echo "-- control again after all hazards cleared"
/pairprobe L eg-wf 1 500 751 1500 751
/pairprobe L eg-sffin 1 500 752 1500 752
cat /proc/ovd_fwd
dmesg | grep -E 'ovdpair|BUG|WARN|Oops|Call Trace' | tail -20
