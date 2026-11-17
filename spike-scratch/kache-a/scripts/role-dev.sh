# Dev-environment VM: prepares its own cargo home, kache install and checkout
# on its data disk, then runs the steps named in $SPIKE_STEPS (scripts/step-*.sh).
mkdir -p "$KACHE_BIN"
cp -u /home/ubuntu/.cargo/bin/sccache "$KACHE_BIN/" 2>/dev/null || true
cp -u "$SPIKE_DIR"/target/vmstate/kache/bin/* "$KACHE_BIN/"
export PATH="$KACHE_BIN:$PATH"
if [ ! -d "$CARGO_HOME/registry/cache" ]; then
  glog "seeding cargo home from host registry"
  mkdir -p "$CARGO_HOME/registry"
  cp -a /home/ubuntu/.cargo/registry/index /home/ubuntu/.cargo/registry/cache "$CARGO_HOME/registry/"
  [ -d /home/ubuntu/.cargo/git ] && cp -a /home/ubuntu/.cargo/git "$CARGO_HOME/"
fi
export CARGO_NET_OFFLINE=true
rc=0
for step in ${SPIKE_STEPS//,/ }; do
  glog "step $step begin"
  ( source "$SPIKE_DIR/scripts/step-$step.sh" ); src=$?
  glog "step $step end rc=$src"
  [ "$src" = 0 ] || rc=1
done
exit "$rc"
