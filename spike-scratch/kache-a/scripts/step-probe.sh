# Probe: the guest sees the host toolchain read-only, reaches Garage, and kache
# restores a unit from the remote into an empty local store.
{
  echo "uname=$(uname -r) nproc=$(nproc)"; free -g | sed -n 2p
  df -h /mnt | tail -1
  rustc -Vv; cargo -V; which mold sccache kache kache-engine
  kache --version; sccache --version
  du -sh /home/ubuntu/.cargo/registry/cache /home/ubuntu/.cargo/registry/index 2>/dev/null
  ls "$SPIKE_SRC/target/bpf" 2>&1
  touch /home/ubuntu/rw-test 2>&1 && echo "HOST ROOT WRITABLE" || echo "host root read-only: ok"
} > "$ME/probe-facts.txt" 2>&1
cat "$ME/probe-facts.txt"
wait_for "$BUS/cache/ready" 300 || { glog "garage never ready"; exit 1; }
load_creds
pfx="probe-$SPIKE_RUN"
arm_env kache "$pfx"
kache doctor --json > "$ME/kache-doctor.json" 2>&1; echo "doctor rc=$?"
kache doctor > "$ME/kache-doctor.txt" 2>&1
p=/mnt/probe-crate; rm -rf "$p"; mkdir -p "$p/src"
printf '[package]\nname="probe"\nversion="0.1.0"\nedition="2021"\n[workspace]\n' > "$p/Cargo.toml"
echo 'fn main(){println!("hi");}' > "$p/src/main.rs"
cd "$p"
fresh_arm_state
sample probe-cold cargo build
snap_stats 1-after-cold kache
cargo clean; stop_wrapper kache
glog "waiting for bucket stats"; sleep 25
tail -12 "$BUS/cache/bucket-stats.log" | tee "$ME/bucket-after-upload.txt"
rm -rf /mnt/kache-store
sample probe-remote-restore cargo build
snap_stats 2-after-restore kache
./target/debug/probe
stop_wrapper kache
cp /mnt/kache-store/kache.log "$ME/kache.log" 2>/dev/null
cp /mnt/home/.cache/kache/kache.log "$ME/kache-home.log" 2>/dev/null
cp /mnt/kache-store/events.jsonl "$ME/events.jsonl" 2>/dev/null
ls -la /mnt/kache-store /mnt/home/.cache/kache 2>&1 | head -30
