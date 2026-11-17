# Shared helpers for guest roles. Sourced by guest-init.sh.
set -uo pipefail
GARAGE="$SPIKE_DIR/staging/garage"
KACHE_BIN=/mnt/kache-bin   # copied from the host-built kache (see role-dev.sh)

wait_for() { # PATH TIMEOUT_S
  local t=0; while [ ! -e "$1" ]; do sleep 1; t=$((t+1)); [ "$t" -lt "$2" ] || return 1; done
}

# sample LABEL CMD...: one timed sample, appended to $ME/samples.tsv, full
# output kept in $ME/logs/<n>-LABEL.log. Never overwrites earlier samples.
sample() {
  local label="$1"; shift
  mkdir -p "$ME/logs"
  local n; n="$(cat "$ME/samples.tsv" 2>/dev/null | wc -l)"; n="$(printf '%03d' "$n")"
  local log="$ME/logs/$n-$label.log"
  local t0 t1 rc
  local j0 j1
  j0="$(awk '/^cpu /{print $2+$3+$4+$7+$8+$9}' /proc/stat)"   # whole-VM busy jiffies: counts daemons too
  t0="$(date +%s.%N)"
  /usr/bin/time -v -o "$log.time" "$@" > "$log" 2>&1; rc=$?
  t1="$(date +%s.%N)"
  j1="$(awk '/^cpu /{print $2+$3+$4+$7+$8+$9}' /proc/stat)"
  local wall; wall="$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.3f", b-a}')"
  local rss cpu_u cpu_s
  rss="$(awk -F': ' '/Maximum resident/{print $2}' "$log.time")"
  cpu_u="$(awk -F': ' '/User time/{print $2}' "$log.time")"
  cpu_s="$(awk -F': ' '/System time/{print $2}' "$log.time")"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%T.%3NZ)" "$SPIKE_NAME" "$n" "$label" \
    "$rc" "$wall" "$cpu_u+$cpu_s" "$rss" "$(( (j1 - j0) / 100 ))" >> "$ME/samples.tsv"
  glog "sample $n $label rc=$rc wall=${wall}s"
  return "$rc"
}


# ---- workspace + arm helpers -------------------------------------------------
# seed_ws DEST: a private copy of this checkout on the guest's own disk.
seed_ws() {
  local d="$1"; mkdir -p "$d"
  rsync -a --delete --exclude=/target --exclude=/.git --exclude=/vendors --exclude=/website \
    --exclude=/docs --exclude=/spike-scratch --exclude=/.context --exclude=node_modules \
    --exclude=/verification --exclude=/perf-baseline --exclude=/mutants-baseline \
    "$SPIKE_SRC/" "$d/"
  mkdir -p "$d/target"
  cp -a "$SPIKE_SRC/target/bpf" "$d/target/bpf"
}
# clean_target DIR: drop everything cargo built, keep the prebuilt BPF objects.
clean_target() { find "$1/target" -mindepth 1 -maxdepth 1 ! -name bpf -exec rm -rf {} +; }

S3_ENDPOINT_URL() { echo "http://10.77.0.2:3900"; }
load_creds() { set -a; source "$BUS/cache/creds.env"; set +a; }

# write_kache_toml PREFIX: kache config; a daemon drops KACHE_S3_* env, so the
# remote lives in the file.
write_kache_toml() {
  cat > /mnt/kache.toml <<CONF
[cache]
local_store = "/mnt/kache-store"
[cache.remote]
type = "s3"
bucket = "kache"
endpoint = "$(S3_ENDPOINT_URL)"
region = "garage"
prefix = "$1"
CONF
}
# arm_env ARM [PREFIX]: plain | plain-noinc | sccache | kache
arm_env() {
  local arm="$1" prefix="${2:-run}"
  unset RUSTC_WRAPPER CARGO_INCREMENTAL KACHE_ADAPTIVE_INCREMENTAL KACHE_CONFIG KACHE_HOST_CONFIG SCCACHE_BUCKET SCCACHE_ENDPOINT \
        SCCACHE_REGION SCCACHE_S3_USE_SSL SCCACHE_S3_KEY_PREFIX SCCACHE_DIR SCCACHE_CACHE_SIZE
  export CARGO_NET_OFFLINE=true
  case "$arm" in
    plain) ;;
    plain-noinc) export CARGO_INCREMENTAL=0 ;;
    sccache)
      export CARGO_INCREMENTAL=0 RUSTC_WRAPPER=sccache SCCACHE_BUCKET=sccache SCCACHE_ENDPOINT="$(S3_ENDPOINT_URL)" \
        SCCACHE_REGION=garage SCCACHE_S3_USE_SSL=false SCCACHE_S3_KEY_PREFIX="$prefix" \
        SCCACHE_S3_SERVER_SIDE_ENCRYPTION=false
      export AWS_ACCESS_KEY_ID="$KACHE_S3_ACCESS_KEY" AWS_SECRET_ACCESS_KEY="$KACHE_S3_SECRET_KEY" ;;
    kache|kache-noadapt|kache-cc)
      write_kache_toml "$prefix"
      export RUSTC_WRAPPER=kache KACHE_CONFIG=/mnt/kache.toml KACHE_HOST_CONFIG="" KACHE_LOG=kache=info
      [ "$arm" = kache-noadapt ] && export KACHE_ADAPTIVE_INCREMENTAL=0
      if [ "$arm" = kache-cc ]; then   # C/C++ objects of build scripts through kache (PATH shims, no `kache init`)
        mkdir -p /mnt/shims; local n; for n in cc gcc c++ g++; do ln -sf "$KACHE_BIN/kache" "/mnt/shims/$n"; done
        export PATH="/mnt/shims:$PATH"
      fi ;;
    *) glog "bad arm $arm"; return 1 ;;
  esac
  return 0
}
# stop_wrapper ARM: flush the wrapper's background uploads.
stop_wrapper() {
  case "$1" in
    kache*) kache daemon stop >>"$ME/kache-daemon-stop.log" 2>&1 ;;
    sccache) sccache --stop-server >>"$ME/sccache-stop.log" 2>&1 ;;
  esac
}
# snap_stats LABEL ARM: wrapper statistics at this point (append-only).
snap_stats() {
  mkdir -p "$ME/stats"
  case "$2" in
    kache*) kache stats --json > "$ME/stats/$1.kache.json" 2>&1 ;;
    sccache) sccache --show-stats --stats-format=json > "$ME/stats/$1.sccache.json" 2>&1 ;;
  esac
}
# fresh_arm_state: empty local stores (remote untouched).
fresh_arm_state() {
  kache daemon stop >/dev/null 2>&1; sccache --stop-server >/dev/null 2>&1
  rm -rf /mnt/kache-store /mnt/home/.cache/kache /mnt/home/.cache/sccache
}
# signal NAME / await NAME TIMEOUT: cross-VM rendezvous on the bus.
signal() { touch "$ME/sig-$1"; }
await() { wait_for "$BUS/$1" "$2" || { glog "timeout awaiting $1"; return 1; }; }

# apply_diff NAME: deterministic synthetic source edits (cumulative).
apply_diff() {
  case "$1" in
    leaf-cli)     printf '\n/// Spike synthetic leaf diff.\npub const SPIKE_LEAF_DIFF: u32 = 1;\n' >> crates/overdrive-cli/src/lib.rs ;;
    leaf-gateway) printf '\n/// Spike synthetic gateway diff.\npub const SPIKE_GATEWAY_DIFF: u32 = 1;\n' >> crates/overdrive-gateway/src/lib.rs ;;
    core)         printf '\n/// Spike synthetic core diff.\npub const SPIKE_CORE_DIFF: u32 = 1;\n' >> crates/overdrive-core/src/lib.rs ;;
    editprobe)    printf '\n/// Spike edit-loop probe: one function body that the loop edits.\n#[must_use]\npub fn spike_edit_probe() -> u64 { 0 } // SPIKE_EDIT\n' >> "$EDIT_FILE" ;;
    *) glog "unknown diff $1"; return 1 ;;
  esac
}
edit_probe() { sed -i "s/{ [0-9]* } \/\/ SPIKE_EDIT/{ $1 } \/\/ SPIKE_EDIT/" "$EDIT_FILE"; }

# flush_wrapper ARM: wait until the wrapper's background uploads are done.
flush_wrapper() {
  case "$1" in
    kache*)
      local i; for i in 1 2 3 4 5 6 7 8; do
        kache daemon stop >>"$ME/kache-daemon-stop.log" 2>&1 && { glog "kache daemon drained (try $i)"; return 0; }
      done
      glog "kache daemon did not drain" ; return 1 ;;
  esac
}

# require_remote: kache fails open on an unreachable remote (it just compiles), so
# a silently dead Garage would invalidate a sample. Fail loudly instead.
require_remote() {
  case "${ARM:-}" in kache*|sccache) ;; *) return 0 ;; esac
  local code; code="$(curl -s -m 5 -o /dev/null -w '%{http_code}' "$(S3_ENDPOINT_URL)/" 2>&1)"
  case "$code" in 200|403|400|404) return 0 ;; esac
  glog "REMOTE-UNREACHABLE http=$code"; echo "REMOTE-UNREACHABLE $(date -u +%FT%T.%3NZ)" >> "$ME/invalid.txt"; exit 9
}
