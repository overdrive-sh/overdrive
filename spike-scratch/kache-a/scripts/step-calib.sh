# Calibration: one cold plain-cargo build of the default workspace target set,
# sized for planning the sample counts.
time seed_ws /mnt/ws
cd /mnt/ws
arm_env plain
du -sh /mnt/ws | tee "$ME/ws-size.txt"
sample s0-plain-inc-cold cargo build --all-targets --locked --offline
du -sh /mnt/ws/target | tee "$ME/target-size.txt"
cargo build --message-format=json --all-targets --locked --offline 2>/dev/null \
  | grep -c compiler-artifact | tee "$ME/artifact-count.txt"
df -h /mnt | tail -1
