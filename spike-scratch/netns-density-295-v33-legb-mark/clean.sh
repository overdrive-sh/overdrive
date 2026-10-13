for p in $(pgrep -x tcpdump) $(pgrep -x v33) $(pgrep -f "[v]33-legb-mark/run.sh"); do kill $p 2>/dev/null; done
sleep 0.3
ip netns del v33ns 2>&1; echo "netns:"; ip netns list; echo "procs:"; ps -eo pid,cmd | grep -E "[v]33 (listen|wild|dial|vm-)|[t]cpdump"; rm -rf /home/marcus.guest/.cargo-target-v33 /home/marcus.guest/.cargo-home-v33; echo clean-done
