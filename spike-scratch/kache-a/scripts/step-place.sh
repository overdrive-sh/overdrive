# S7: placement-record comparison. Builds the same test binaries with plain
# cargo (incremental, the repo default) or through kache, then keeps each test
# binary's overdrive_test_placement section bytes for an offline byte/record compare.
load_creds
seed_ws /mnt/ws
cd /mnt/ws
arm_env "$ARM" "$PFX"
fresh_arm_state
BUILD=(cargo build -p overdrive-gateway -p overdrive-host -p overdrive-testing --tests --locked --offline)
sample "place-$ARM" "${BUILD[@]}"
"${BUILD[@]}" --message-format=json 2>/dev/null | grep '"reason":"compiler-artifact"' > "$ME/artifacts-place.jsonl"
python3 "$SPIKE_DIR/scripts/manifest.py" "$ME/artifacts-place.jsonl" > "$ME/manifest-place.tsv"
mkdir -p "$ME/sections"
python3 - "$ME/artifacts-place.jsonl" "$ME/sections" <<'PY'
import json, subprocess, sys, os
src, out = sys.argv[1], sys.argv[2]
for line in open(src):
    m = json.loads(line)
    exe = m.get("executable")
    if not exe:
        continue
    name = m["package_id"].split("#")[-1].split(" ")[0] + "-" + m["target"]["name"] + "-" + ",".join(m["target"]["kind"])
    dst = os.path.join(out, name.replace("/", "_") + ".bin")
    r = subprocess.run(["objcopy", "-O", "binary", "--only-section=overdrive_test_placement", exe, dst], capture_output=True)
    if r.returncode != 0 or os.path.getsize(dst) == 0:
        os.path.exists(dst) and os.remove(dst)
PY
gzip -f "$ME/artifacts-place.jsonl"
flush_wrapper "$ARM"
case "$ARM" in kache*) cp -f /mnt/kache-store/events.jsonl "$ME/kache-events.jsonl" 2>/dev/null ;; esac
ls -la "$ME/sections" | head -30
