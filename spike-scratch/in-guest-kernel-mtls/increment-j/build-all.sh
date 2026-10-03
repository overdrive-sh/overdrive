#!/usr/bin/env bash
# Spike D (GH #303, increment-i) BUILD phase. Runs as the LOGIN USER
# (cargo xtask metal run --no-sudo) so every build output in the unsynced
# scratch tree is login-user-owned. Builds, in order: the host rustls programs
# (relay resolve/LB + two labelled backends) + PKI, the out-of-tree kernel
# module against the stock kernel headers, the static guest app, and the mesh
# initramfs. Stages NO kernel and boots NOTHING -- that is run-mesh-fc.sh (root).
# No sudo, no system-package installs.
set -euo pipefail
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "##### [build-all] host rustls programs + PKI (relay resolve/LB, two backends)"
bash "$INC/build-host.sh"
echo "##### [build-all] out-of-tree kernel module igkmd_mtls.ko"
bash "$INC/build-module.sh" igkmd_mtls
echo "##### [build-all] static guest app igkmd_app"
bash "$INC/build-binary.sh"
echo "##### [build-all] mesh initramfs"
bash "$INC/build-guest.sh" mesh

OUT="$HOME/igkm-spike-a/igkmd/out"
BIN="$HOME/igkm-spike-a/igkmd-host-target/release"
echo "##### [build-all] artifact summary (sha256)"
sha256sum "$OUT/module/igkmd_mtls.ko" "$OUT/initramfs-mesh.cpio.gz" "$OUT/bin/igkmd_app" \
          "$BIN/peer" "$BIN/relay" 2>&1
echo "##### [build-all] ownership (must be the login user, NOT root)"
ls -l "$OUT/module/igkmd_mtls.ko" "$OUT/initramfs-mesh.cpio.gz" "$BIN/peer"
echo "##### [build-all] DONE"
