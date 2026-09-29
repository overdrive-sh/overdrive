#!/usr/bin/env bash
# Spike B3 stretch (GH #303, increment-f): build the Go HTTP client
# (app-go/main.go, standard library only) as a static-pie ELF with an official
# Go toolchain downloaded into the scratch tree (sha256 from go.dev's release
# JSON). Metal login user, NO sudo, NO system packages.
#
# Output: ./out/bin/igkmf-go (gitignored). Scratch: ~/igkm-spike-a/stretch/.
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly S="$HOME/igkm-spike-a/stretch"
readonly OUTD="$INCREMENT/out/bin"

step() { printf '\n##### [build-go] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

step "latest stable Go for linux-amd64 from https://go.dev/dl/?mode=json"
mkdir -p "$S"
cd "$S"
curl -fsSL -o go-dl.json 'https://go.dev/dl/?mode=json'
read -r GOVER GOFILE GOSHA < <(python3 - <<'PY'
import json
rel = json.load(open("go-dl.json"))
r = next(x for x in rel if x.get("stable"))
f = next(x for x in r["files"] if x["os"] == "linux" and x["arch"] == "amd64" and x["kind"] == "archive")
print(r["version"], f["filename"], f["sha256"])
PY
)
echo "version=$GOVER file=$GOFILE sha256(go.dev)=$GOSHA"
[[ -f "$GOFILE" ]] || curl -fsSL -o "$GOFILE" "https://go.dev/dl/$GOFILE"
echo "$GOSHA  $GOFILE" | sha256sum -c -
if [[ ! -x "$S/$GOVER/go/bin/go" ]]; then
  rm -rf "$S/$GOVER"; mkdir -p "$S/$GOVER"
  tar -C "$S/$GOVER" -xzf "$GOFILE"
fi
export GOROOT="$S/$GOVER/go" PATH="$S/$GOVER/go/bin:$PATH"
export GOPATH="$S/gopath" GOCACHE="$S/gocache" GOMODCACHE="$S/gopath/pkg/mod"
export GOFLAGS=-mod=mod GOTOOLCHAIN=local GOPROXY=off GOTELEMETRY=off
go version
go env GOOS GOARCH CC

step "record: CGO_ENABLED=0 -buildmode=pie with Go's internal linker (what does it produce?)"
cd "$INCREMENT/app-go"
sha256sum main.go go.mod
mkdir -p "$OUTD" "$S/go-internal-pie"
CGO_ENABLED=0 go build -trimpath -buildmode=pie -o "$S/go-internal-pie/igkmf-go" . 2>&1
file "$S/go-internal-pie/igkmf-go"
readelf -lW "$S/go-internal-pie/igkmf-go" | grep -E '^\s+(INTERP|DYNAMIC)|Requesting' || true
echo "(run 0018: CGO_ENABLED=0 with -linkmode=external is refused by go1.27.1:"
echo " '-linkmode=external requires external (cgo) linking, but cgo is not enabled')"

step "build: CGO_ENABLED=1 (runtime/cgo, static glibc), -buildmode=pie, external link with -static-pie, netgo+osusergo"
set +e
CGO_ENABLED=1 go build -trimpath -buildmode=pie -tags netgo,osusergo \
  -ldflags='-linkmode=external -extldflags=-static-pie' -o "$OUTD/igkmf-go" . 2>&1
rc=$?
set -e
echo "go build exit=$rc"
[[ "$rc" -eq 0 ]] || exit "$rc"

step "identity"
file "$OUTD/igkmf-go"
sha256sum "$OUTD/igkmf-go"
ls -l "$OUTD/igkmf-go"
readelf -h "$OUTD/igkmf-go" | grep -E 'Type|Entry'
readelf -lW "$OUTD/igkmf-go" | grep -E '^\s+(INTERP|DYNAMIC|TLS)' || true
readelf -dW "$OUTD/igkmf-go" | grep NEEDED || echo "  NEEDED: (none)"
go version -m "$OUTD/igkmf-go" | head -12

step "native smoke test on the host Linux (no argument: banner only, exit 2)"
set +e
"$OUTD/igkmf-go"
echo "exit=$?"
