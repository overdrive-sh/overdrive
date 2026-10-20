echo "== F3: hazard variants + real-vhost guest pairs against the host userspace peer"
P=/sys/module/ovdpair/parameters
uname -r
ls $P
/pairprobe probe
cat /proc/ovd_fwd
echo "-- control (all rules on), 500 trials"
/pairprobe L eg-wf 1 500 1701 1500 1701
/pairprobe L eg-sffin 1 500 1702 1500 1702
/pairprobe L in-cfearly 1 500 1703 1500 1703
/pairprobe L in-cf 1 500 1704 1500 1704
echo "-- h_strict=1: attach requires TCP_ESTABLISHED (the evmod spike check)"
echo 1 > $P/h_strict; echo "set h_strict=$(cat $P/h_strict)"
/pairprobe L eg-wf 1 500 1711 1500 1711
/pairprobe L in-sffin 1 500 1712 1500 1712
echo 0 > $P/h_strict
echo "-- h_noseed=1: no attach-time classification of a vsock end already received"
echo 1 > $P/h_noseed; echo "set h_noseed=$(cat $P/h_noseed)"
/pairprobe L eg-sffin 1 500 1721 1500 1721
/pairprobe L in-cfearly 1 500 1722 1500 1722
/pairprobe L eg-wf 1 500 1723 1500 1723
echo 0 > $P/h_noseed
echo "-- h_fault=1: one byte dropped by the forwarder (the probe must see it)"
echo 1 > $P/h_fault; echo "set h_fault=$(cat $P/h_fault)"
/pairprobe L eg-wf 1 200 1731 1500 1731
/pairprobe L in-cf 1 200 1732 1500 1732
echo 0 > $P/h_fault
echo "-- h_nokick=1: no attach-time run"
echo 1 > $P/h_nokick; echo "set h_nokick=$(cat $P/h_nokick)"
/pairprobe L eg-wf 1 20 1741 600 1741
/pairprobe L in-sf 1 20 1742 600 1742
/pairprobe L in-cfearly 1 20 1743 600 1743
echo 0 > $P/h_nokick
echo "-- h_lateshut=1: own_wr_shut published after the SHUTDOWN leaves (window widened)"
echo 1 > $P/h_lateshut; echo "set h_lateshut=$(cat $P/h_lateshut)"
/pairprobe L in-cf 1 300 1751 1500 1751
/pairprobe L in-sf 1 300 1752 1500 1752
/pairprobe L eg-wf 1 300 1753 1500 1753
echo 0 > $P/h_lateshut
echo "-- control again after all hazards cleared"
/pairprobe L eg-wf 1 500 1761 1500 1761
/pairprobe L eg-sffin 1 500 1762 1500 1762
/pairprobe L in-cf 1 500 1763 1500 1763
cat /proc/ovd_fwd
echo "-- eg-drst on the final build (F1 ran it with a probe verdict bug: a failed send consumes sk_err, the recv then reads EOF)"
/pairprobe L eg-drst 1 10000 1771 8000 1771
/pairprobe L eg-drst 8 1250 1772 8000 1772
/pairprobe L eg-drst 8 10000 1773 8000 1773
echo "-- real vhost-vsock: guest pairs against the host peer"
i=0
for s in eg-wf eg-sf eg-sffin eg-nofin eg-drst in-sf in-sffin in-cf in-cfearly; do
  i=$((i+1))
  /pairprobe G $s 1 10000 $((1800+i)) 8000 $((1800+i)) 2 3371
done
/pairprobe G eg-rst 1 2000 1810 8000 1810 2 3371
/pairprobe G eg-wf 8 1250 1821 8000 1821 2 3371
/pairprobe G eg-drst 8 1250 1822 8000 1822 2 3371
/pairprobe G in-cfearly 8 1250 1823 8000 1823 2 3371
/pairprobe G in-cf 8 10000 1824 8000 1824 2 3371
grep -E 'MemAvailable|SUnreclaim|Slab' /proc/meminfo
cat /proc/ovd_fwd
dmesg | grep -E 'ovdpair|BUG|WARN|Oops|Call Trace' | tail -20
