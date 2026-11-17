# Commit ladder for one arm in this VM (S0-S3, S5, S10, S4 edit loop, S7 evidence).
#   ARM   plain | plain-noinc | sccache | kache | kache-noadapt
#   PFX   remote key prefix          ROLE  A | B | C | D   (label only)
#   WS    checkout path (default /mnt/ws)
#   FRESH X | Xm : a fresh VM building ONE state (X, or X plus all three diffs)
#         unset    : the full ladder X -> +cli -> +gateway -> +core
#   LOOP  edit-loop iterations after the ladder (0 = none)
#   TESTS 1 = also run the default-lane tests of overdrive-core at the end
WS="${WS:-/mnt/ws}"
LOOP="${LOOP:-0}"
TESTS="${TESTS:-0}"
FRESH="${FRESH:-}"
{ echo "arm=$ARM role=$ROLE pfx=$PFX ws=$WS fresh=$FRESH loop=$LOOP tests=$TESTS smoke=${SMOKE:-0}"
  uname -r; nproc; free -g | sed -n 2p; } > "$ME/ladder-facts.txt"
load_creds
seed_ws "$WS"
cd "$WS"
arm_env "$ARM" "$PFX"
fresh_arm_state
BUILD=(cargo build --all-targets --locked --offline)
EDIT_BUILD=(cargo build -p overdrive-control-plane --test acceptance --locked --offline)
EDIT_FILE=crates/overdrive-control-plane/src/lib.rs
if [ "${SMOKE:-0}" = 1 ]; then   # plumbing check only: one small crate
  BUILD=(cargo build -p overdrive-core --lib --locked --offline)
  EDIT_BUILD=(cargo build -p overdrive-core --lib --locked --offline)
  EDIT_FILE=crates/overdrive-core/src/lib.rs
fi
export EDIT_FILE

# one measured state: build, then the evidence for it.
state() {
  local label="$1"
  require_remote
  sample "$label" "${BUILD[@]}"
  local rc=$?
  snap_stats "$label" "$ARM"
  # Artifact list (kind, fresh) of the tree, plus hashes (S7, S9).
  "${BUILD[@]}" --message-format=json 2>/dev/null | grep '"reason":"compiler-artifact"' > "$ME/artifacts-$label.jsonl"
  python3 "$SPIKE_DIR/scripts/manifest.py" "$ME/artifacts-$label.jsonl" > "$ME/manifest-$label.tsv"
  gzip -f "$ME/artifacts-$label.jsonl"
  du -sb "$WS/target" | awk '{print $1}' > "$ME/target-bytes-$label.txt"
  return "$rc"
}
# remap_check LABEL: embedded absolute paths in a representative test binary.
remap_check() {
  local exe; exe="$(awk -F'\t' '$1=="test" && $2 ~ /overdrive-core/ && $3=="acceptance" {print $5}' "$ME/manifest-$1.tsv" | head -1)"
  [ -n "$exe" ] || return 0
  { echo "exe=$exe"; echo "checkout_path_occurrences=$(strings -a "$exe" | grep -c "$WS")"
    echo "kache_sentinel_occurrences=$(strings -a "$exe" | grep -c '/kache/')"; } > "$ME/remap-$1.txt"
}

if [ -n "$FRESH" ]; then
  if [ "$FRESH" = Xm ]; then apply_diff leaf-cli; apply_diff leaf-gateway; apply_diff core; fi
  state "fresh-$FRESH"; remap_check "fresh-$FRESH"
else
  state s1-X
  apply_diff leaf-cli;      state s2-leaf-cli
  apply_diff leaf-gateway;  state s2b-gateway
  apply_diff core;          state s3-core
  remap_check s3-core
fi

flush_wrapper "$ARM"
snap_stats final "$ARM"
case "$ARM" in
  kache*)
    cp -f /mnt/kache-store/events.jsonl "$ME/kache-events.jsonl" 2>/dev/null
    cp -f /mnt/kache-store/daemon.log "$ME/kache-daemon.log" 2>/dev/null
    cp -f /mnt/kache-store/kache.log "$ME/kache.log" 2>/dev/null
    du -sb /mnt/kache-store > "$ME/kache-store-bytes.txt" ;;
  sccache) sccache --show-stats > "$ME/sccache-stats.txt" 2>&1 ;;
esac

if [ "$TESTS" = 1 ]; then
  glog "S7: default-lane tests (overdrive-core)"
  sample s7-tests-core cargo nextest run -p overdrive-core --locked --offline --no-fail-fast
fi

if [ "$LOOP" -gt 0 ]; then
  glog "S4: edit loop x$LOOP"
  apply_diff editprobe
  sample edit-setup "${EDIT_BUILD[@]}"
  for i in $(seq "$LOOP"); do
    edit_probe "$i"
    sample "edit-$i" "${EDIT_BUILD[@]}"
  done
  snap_stats after-loop "$ARM"
  flush_wrapper "$ARM"
  case "$ARM" in kache*) cp -f /mnt/kache-store/events.jsonl "$ME/kache-events-after-loop.jsonl" 2>/dev/null ;; esac
fi
df -h /mnt | tail -1 > "$ME/df-end.txt"
