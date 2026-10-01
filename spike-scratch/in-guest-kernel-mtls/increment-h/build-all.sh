#!/usr/bin/env bash
# Spike C re-run (GH #303, increment-h) BUILD phase. Runs as the LOGIN USER
# (cargo xtask metal run --no-sudo) so every build output in the unsynced
# scratch tree is login-user-owned (building as root once broke a later rsync).
# Builds, in order: the modified host rustls programs (+ PKI), the out-of-tree
# kernel module against the stock kernel headers, the static guest app, and the
# mesh initramfs. Stages NO kernel and boots NOTHING -- that is run-mesh-fc.sh
# (root). No sudo, no system-package installs.
set -euo pipefail
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "##### [build-all] host rustls programs + PKI (Change 1: peer tickets=0)"
bash "$INC/build-host.sh"
echo "##### [build-all] out-of-tree kernel module igkmh_mtls.ko"
bash "$INC/build-module.sh" igkmh_mtls
echo "##### [build-all] static guest app igkmh_app"
bash "$INC/build-binary.sh"
echo "##### [build-all] mesh initramfs"
bash "$INC/build-guest.sh" mesh

OUT="$HOME/igkm-spike-a/igkmh/out"
BIN="$HOME/igkm-spike-a/igkmh-host-target/release"
echo "##### [build-all] artifact summary (sha256)"
sha256sum "$OUT/module/igkmh_mtls.ko" "$OUT/initramfs-mesh.cpio.gz" "$OUT/bin/igkmh_app" \
          "$BIN/peer" "$BIN/relay" 2>&1
echo "##### [build-all] ownership (must be the login user, NOT root)"
ls -l "$OUT/module/igkmh_mtls.ko" "$OUT/initramfs-mesh.cpio.gz" "$BIN/peer"
echo "##### [build-all] DONE"
