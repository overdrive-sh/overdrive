# S6 concurrency: N VMs build the same tree through kache at the same time.
#   PEERS comma list of VM names in this wave (barrier on their ready files)
#   WAVE  label        PFX remote prefix       SPEC full | core
load_creds
ls /home/ubuntu/.rustup/toolchains > "$ME/toolchains.txt" 2>&1
seed_ws /mnt/ws
cd /mnt/ws
arm_env kache "$PFX"
fresh_arm_state
case "$SPEC" in
  full) BUILD=(cargo build --all-targets --locked --offline) ;;
  core) BUILD=(cargo build -p overdrive-core --all-targets --locked --offline) ;;
esac
{ echo "wave=$WAVE spec=$SPEC pfx=$PFX peers=$PEERS"; uname -r; nproc; free -g | sed -n 2p; } > "$ME/conc-facts.txt"
touch "$ME/ready"
for p in ${PEERS//,/ }; do await "$p/ready" 300 || exit 1; done
glog "barrier passed: all peers ready"
sample "conc-$WAVE" "${BUILD[@]}"
snap_stats "conc-$WAVE" kache
"${BUILD[@]}" --message-format=json 2>/dev/null | grep '"reason":"compiler-artifact"' > "$ME/artifacts-conc-$WAVE.jsonl"
python3 "$SPIKE_DIR/scripts/manifest.py" "$ME/artifacts-conc-$WAVE.jsonl" > "$ME/manifest-conc-$WAVE.tsv"
gzip -f "$ME/artifacts-conc-$WAVE.jsonl"
flush_wrapper kache
cp -f /mnt/kache-store/events.jsonl "$ME/kache-events.jsonl" 2>/dev/null
cp -f /mnt/kache-store/daemon.log "$ME/kache-daemon.log" 2>/dev/null
cp -f /mnt/kache-store/kache.log "$ME/kache.log" 2>/dev/null
du -sb /mnt/kache-store > "$ME/kache-store-bytes.txt"
# Failure signatures named in kache issues (index.db lock, truncated archive, ...).
grep -hiE "index\.db|unable to open|corrupt|truncated|database is locked|checksum|mismatch|panicked|ERROR" \
  "$ME/kache-daemon.log" "$ME/kache.log" "/mnt/kache-store/kache.log" 2>/dev/null | sort | uniq -c | sort -rn | head -40 > "$ME/failure-signatures.txt"
df -h /mnt | tail -1 > "$ME/df-end.txt"
