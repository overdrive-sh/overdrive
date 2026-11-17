# S4 paired edit loop. Two checkouts in ONE VM, one plain (incremental, the repo
# default) and one through kache (remote alive, adaptive default), edited and
# rebuilt in alternation so box load drift hits both alike. The order inside each
# pair flips every iteration.
#   MODE publish : build X through kache with KACHE_ADAPTIVE_INCREMENTAL=0 (publisher VM)
#   MODE paired  : plain builds X cold; kache RESTORES X from the remote, then both
#                  take the first edit (setup) and N alternating edits.
N="${N:-12}"
PFX=s4-1
load_creds
EDIT_FILE=crates/overdrive-control-plane/src/lib.rs
export EDIT_FILE
EB=(cargo build -p overdrive-control-plane --test acceptance --locked --offline)
XB=(cargo build --all-targets --locked --offline)
PROBE='\n/// Spike edit-loop probe: one function body that the loop edits.\n#[must_use]\npub fn spike_edit_probe() -> u64 { 0 } // SPIKE_EDIT\n'

if [ "$MODE" = publish ]; then
  seed_ws /mnt/ws-kache
  cd /mnt/ws-kache
  arm_env kache-noadapt "$PFX"
  fresh_arm_state
  require_remote
  sample s4-publish-x "${XB[@]}"
  flush_wrapper kache-noadapt
  exit 0
fi

seed_ws /mnt/ws-plain
seed_ws /mnt/ws-kache
fresh_arm_state
( cd /mnt/ws-plain && arm_env plain; sample s4-x-plain "${XB[@]}" )
( cd /mnt/ws-kache && arm_env kache "$PFX"; ARM=kache require_remote; sample s4-x-kache-restore "${XB[@]}" )
( cd /mnt/ws-plain && printf "$PROBE" >> "$EDIT_FILE" )
( cd /mnt/ws-kache && printf "$PROBE" >> "$EDIT_FILE" )
( cd /mnt/ws-plain && arm_env plain; sample s4-setup-plain "${EB[@]}" )
( cd /mnt/ws-kache && arm_env kache "$PFX"; sample s4-setup-kache "${EB[@]}" )
for i in $(seq "$N"); do
  if [ $((i % 2)) = 0 ]; then order="plain kache"; else order="kache plain"; fi
  for arm in $order; do
    ( cd "/mnt/ws-$arm"
      edit_probe "$i"
      if [ "$arm" = kache ]; then arm_env kache "$PFX"; else arm_env plain; fi
      sample "s4-edit-$arm-$i" "${EB[@]}" )
  done
done
( cd /mnt/ws-kache && arm_env kache "$PFX"; snap_stats s4-final kache; flush_wrapper kache )
cp -f /mnt/kache-store/events.jsonl "$ME/kache-events.jsonl" 2>/dev/null
df -h /mnt | tail -1 > "$ME/df-end.txt"
