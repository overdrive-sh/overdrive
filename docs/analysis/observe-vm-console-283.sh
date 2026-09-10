#!/usr/bin/env bash
# Read-only sampling of this example's exact allocation console paths.
# Every changed snapshot has a fresh path; no previous snapshot is overwritten.
set -euo pipefail
dir="$1"
mkdir "$dir/consoles"
for index in $(seq 1 1200); do
  [[ ! -f "$dir/observer-done" ]] || break
  for alloc in alloc-service-vm-e08-0 alloc-service-vm-e08-client-0; do
    source="/run/overdrive/vm/$alloc/console.log"
    [[ -f "$source" ]] || continue
    # No state outside the capture directory is touched.
    if [[ ! -f "$dir/consoles/$alloc.last" ]] || ! cmp -s "$source" "$dir/consoles/$alloc.last"; then
      cp "$source" "$dir/consoles/$alloc-$index.log" || continue
      # last is an index link, not evidence; its targets remain append-only.
      ln -sf "$alloc-$index.log" "$dir/consoles/$alloc.last"
      printf '%s %s %s\n' "$(date -u +%FT%T.%NZ)" "$alloc" "$index" >>"$dir/console-index"
    fi
  done
  sleep 0.1
done
