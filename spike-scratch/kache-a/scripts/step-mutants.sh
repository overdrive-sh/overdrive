# S8: cargo-mutants on one small file of overdrive-core, --jobs 4, with the
# wrapper arm named in ARM. cargo-mutants builds each worker in its own tree copy.
load_creds
seed_ws /mnt/ws
cd /mnt/ws
arm_env "$ARM" "$PFX"
fresh_arm_state
mkdir -p /mnt/tmp; export TMPDIR=/mnt/tmp
export OVERDRIVE_BPF_OBJECT=/mnt/ws/target/bpf/overdrive_bpf.o
cargo mutants --version | tee "$ME/mutants-version.txt"
FILE=""
for f in crates/overdrive-core/src/*.rs; do
  n="$(cargo mutants -p overdrive-core --file "$f" --list 2>/dev/null | wc -l)"
  echo "$f $n" >> "$ME/mutants-candidates.txt"
  if [ "$n" -ge 14 ] && [ "$n" -le 36 ]; then FILE="$f"; break; fi
done
[ -n "$FILE" ] || { glog "no suitable file"; exit 1; }
echo "$FILE" > "$ME/mutants-file.txt"
sample "mutants-$ARM" cargo mutants -p overdrive-core --file "$FILE" --jobs 4 --test-tool nextest \
  --timeout 600 --build-timeout 900 --output "$ME/mutants-out" --no-shuffle
snap_stats mutants "$ARM"
flush_wrapper "$ARM"
case "$ARM" in kache*)
  cp -f /mnt/kache-store/events.jsonl "$ME/kache-events.jsonl" 2>/dev/null
  cp -f /mnt/kache-store/daemon.log "$ME/kache-daemon.log" 2>/dev/null ;; esac
cp -r "$ME/mutants-out/mutants.out/outcomes.json" "$ME/outcomes.json" 2>/dev/null
df -h /mnt | tail -1 > "$ME/df-end.txt"
