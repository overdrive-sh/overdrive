# Diagnostic: list what the remote checkout holds.
plan_main() {
  ls -la "$SPIKE/evidence" "$STATE" "$STATE/bus" 2>&1 | sed 's/^/ls: /' >&2
  du -sh "$STATE"/* 2>&1 | sed 's/^/du: /' >&2
}
