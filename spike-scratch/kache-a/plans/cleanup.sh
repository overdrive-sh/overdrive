# Final cleanup of the spike's state on the box (disks, buses, built kache, sockets).
plan_main() {
  df -h "$STATE" | tail -1 | sed 's/^/before: /' >&2
  du -sh "$STATE"/* 2>/dev/null | sed 's/^/before: /' >&2
  rm -rf "$STATE/disks" "$STATE/bus" "$STATE/kache" "$STATE/kache-src" "$STATE/sock" "$STATE/evidence-archive" \
         "$STATE"/initramfs-*.cpio.gz
  rm -rf "$SPIKE/target/x86_64-unknown-linux-musl" "$SPIKE/target/release" "$SPIKE/target/debug"
  df -h "$STATE" | tail -1 | sed 's/^/after: /' >&2
  du -sh "$SPIKE/target" 2>/dev/null | sed 's/^/after: spike target /' >&2
}
