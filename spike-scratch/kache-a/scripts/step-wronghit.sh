# S7/S9 wrong-hit probes: inputs kache cannot see through rustc args alone.
# overdrive-dataplane embeds target/bpf/overdrive_bpf.o with include_bytes! and
# its build script re-exports absolute paths. Each build must contain exactly the
# object that was current, never a restored older variant.
#   MODE publish : build the pristine tree, publish.
#   MODE probe   : run the mutation sequence against the warm remote.
#   MODE fresh   : a fresh VM that must restore variant ONE from the remote.
load_creds
seed_ws /mnt/ws
cd /mnt/ws
arm_env kache "$PFX"
fresh_arm_state
OBJ=target/bpf/overdrive_bpf.o
cp "$OBJ" /mnt/obj-orig
M1=SPIKEMARKER_ONE_0123456789ABCDEF
M2=SPIKEMARKER_TWO_0123456789ABCDEF
M3=SPIKEMARKER_THREE_0123456789ABC
BUILD=(cargo build -p overdrive-dataplane --lib --locked --offline)
verdicts="$ME/wronghit-verdicts.tsv"
: > "$verdicts"

# build_check LABEL EXPECT_PRESENT... -- ABSENT...: marker presence in the produced rlib.
build_check() {
  local label="$1"; shift
  local present=() absent=() mode=present m
  for m in "$@"; do [ "$m" = -- ] && { mode=absent; continue; }; [ "$mode" = present ] && present+=("$m") || absent+=("$m"); done
  sample "$label" "${BUILD[@]}"
  local rlib; rlib="$(ls -t target/debug/deps/liboverdrive_dataplane-*.rlib | head -1)"
  local ok=pass
  for m in "${present[@]}"; do [ "$(grep -ac "$m" "$rlib")" -gt 0 ] || { ok=WRONG-HIT-missing-$m; }; done
  for m in "${absent[@]}"; do [ "$(grep -ac "$m" "$rlib")" -eq 0 ] || { ok=WRONG-HIT-stale-$m; }; done
  printf '%s\t%s\t%s\n' "$label" "$ok" "$rlib" >> "$verdicts"
  snap_stats "$label" kache
  glog "wronghit $label => $ok"
}

case "$MODE" in
  publish)
    build_check w0-pristine -- "$M1" "$M2" "$M3"
    flush_wrapper kache ;;
  probe)
    build_check w1-restore-pristine -- "$M1" "$M2" "$M3"
    printf '%s' "$M1" >> "$OBJ";                       build_check w2-object-ONE "$M1" -- "$M2" "$M3"
    cp /mnt/obj-orig "$OBJ"; touch "$OBJ";             build_check w3-object-restored -- "$M1" "$M2" "$M3"
    cp /mnt/obj-orig "$OBJ"; printf '%s' "$M2" >> "$OBJ"
                                                       build_check w4-object-TWO-same-length "$M2" -- "$M1" "$M3"
    cp /mnt/obj-orig /mnt/obj-orig-copy
    mkdir -p /mnt/alt; cp /mnt/obj-orig /mnt/alt/overdrive_bpf.o; printf '%s' "$M3" >> /mnt/alt/overdrive_bpf.o
    OVERDRIVE_BPF_OBJECT=/mnt/alt/overdrive_bpf.o build_check w5-env-override-THREE "$M3" -- "$M1" "$M2"
    cp /mnt/obj-orig "$OBJ"
    build_check w6-env-unset-pristine -- "$M1" "$M2" "$M3"
    # Variant ONE rebuilt last so the remote holds it for the fresh VM.
    printf '%s' "$M1" >> "$OBJ";                       build_check w7-object-ONE-again "$M1" -- "$M2" "$M3"
    flush_wrapper kache ;;
  fresh)
    printf '%s' "$M1" >> "$OBJ";                       build_check w8-fresh-vm-object-ONE "$M1" -- "$M2" "$M3"
    cp /mnt/obj-orig "$OBJ";                           build_check w9-fresh-vm-pristine -- "$M1" "$M2" "$M3" ;;
esac
cp -f /mnt/kache-store/events.jsonl "$ME/kache-events.jsonl" 2>/dev/null
cp -f /mnt/kache-store/daemon.log "$ME/kache-daemon.log" 2>/dev/null
cat "$verdicts"
