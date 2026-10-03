#!/usr/bin/env bash
# Spike A3 prep (metal login user, NO sudo, NO system packages).
#
# The Unikraft make/Kconfig build compiles its Kconfig `conf` tool from
# lexer.l/parser.y on a Linux host (unikraft:Makefile:1000-1040,
# support/kconfig/Makefile.rules:64-84), so it needs flex and bison; both need
# m4. The metal host has none of the three (run 0001), so this builds them from
# upstream source tarballs into a user-owned prefix outside the rsynced tree:
#
#   $HOME/igkm-spike-a/unikraft/tools/{src,prefix}
#
# It also clones Unikraft core and lib-lwip at the pinned SHAs into
# $HOME/igkm-spike-a/unikraft/{unikraft,lib-lwip} and verifies HEAD and the
# tree object. Every step is skipped when its output already exists.
set -euo pipefail

readonly W="$HOME/igkm-spike-a/unikraft"
readonly SRC="$W/tools/src"
readonly PREFIX="$W/tools/prefix"
readonly UK_SHA=eb8fa2368618cea11c9bde196f79e6e6b9caeed5
readonly LWIP_SHA=ec55ae17618feeb57c8c10109bcf5c42723e8e95
# Tree objects of the laptop read copies at the same SHAs (git rev-parse HEAD^{tree}),
# so the metal build compiles exactly the source the findings cite.
readonly UK_TREE=48639ed4fe5ce6133789c54a043d1ae86fed4f61
readonly LWIP_TREE=16e828f2b3af7b73787307189f624538cffd8974

readonly M4_VER=1.4.19
readonly BISON_VER=3.8.2
readonly FLEX_VER=2.6.4
readonly M4_URL="https://ftp.gnu.org/gnu/m4/m4-$M4_VER.tar.xz"
readonly BISON_URL="https://ftp.gnu.org/gnu/bison/bison-$BISON_VER.tar.xz"
readonly FLEX_URL="https://github.com/westes/flex/releases/download/v$FLEX_VER/flex-$FLEX_VER.tar.gz"

step() { printf '\n##### [tools] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

mkdir -p "$SRC" "$PREFIX"
export PATH="$PREFIX/bin:$PATH"
# gcc 15 defaults to C23; build these older autotools packages as gnu17.
export CFLAGS="-O2 -std=gnu17"

fetch() { # url -> file in $SRC; prints sha256
  local url="$1" f
  f="$SRC/$(basename "$url")"
  [[ -f "$f" ]] || curl -fsSL -o "$f" "$url"
  sha256sum "$f"
}

step "GNU keyring (for .sig verification of m4/bison)"
[[ -f "$SRC/gnu-keyring.gpg" ]] || curl -fsSL -o "$SRC/gnu-keyring.gpg" https://ftp.gnu.org/gnu/gnu-keyring.gpg
sha256sum "$SRC/gnu-keyring.gpg"
command -v gpgv || echo "gpgv MISSING"

build_autotools() { # name ver url
  local name="$1" ver="$2" url="$3" f dir
  step "$name $ver"
  if [[ -x "$PREFIX/bin/$name" ]]; then
    echo "already built: $("$PREFIX/bin/$name" --version | head -1)"
    fetch "$url"
    return
  fi
  fetch "$url"
  f="$SRC/$(basename "$url")"
  dir="$SRC/$name-$ver"
  rm -rf "$dir"
  tar -xf "$f" -C "$SRC"
  cd "$dir"
  ./configure --prefix="$PREFIX" --disable-nls ${4:-} > "$W/tools/$name-configure.log" 2>&1 || {
    echo "configure FAILED"; tail -n 40 "$W/tools/$name-configure.log"; exit 1; }
  make -j"$(nproc)" > "$W/tools/$name-make.log" 2>&1 || {
    echo "make FAILED"; tail -n 60 "$W/tools/$name-make.log"; exit 1; }
  make install > "$W/tools/$name-install.log" 2>&1 || {
    echo "install FAILED"; tail -n 40 "$W/tools/$name-install.log"; exit 1; }
  "$PREFIX/bin/$name" --version | head -1
}

build_autotools m4 "$M4_VER" "$M4_URL"
step "verify m4 signature"
[[ -f "$SRC/m4-$M4_VER.tar.xz.sig" ]] || curl -fsSL -o "$SRC/m4-$M4_VER.tar.xz.sig" "$M4_URL.sig"
gpgv --keyring "$SRC/gnu-keyring.gpg" "$SRC/m4-$M4_VER.tar.xz.sig" "$SRC/m4-$M4_VER.tar.xz" 2>&1 || echo "m4 signature: NOT VERIFIED"

build_autotools bison "$BISON_VER" "$BISON_URL" "M4=$PREFIX/bin/m4"
step "verify bison signature"
[[ -f "$SRC/bison-$BISON_VER.tar.xz.sig" ]] || curl -fsSL -o "$SRC/bison-$BISON_VER.tar.xz.sig" "$BISON_URL.sig"
gpgv --keyring "$SRC/gnu-keyring.gpg" "$SRC/bison-$BISON_VER.tar.xz.sig" "$SRC/bison-$BISON_VER.tar.xz" 2>&1 || echo "bison signature: NOT VERIFIED"

build_autotools flex "$FLEX_VER" "$FLEX_URL" "M4=$PREFIX/bin/m4"
step "compare flex tarball with the GitHub release asset digest"
curl -fsSL https://api.github.com/repos/westes/flex/releases/tags/v$FLEX_VER \
  | python3 -c 'import json,sys; [print(a["name"], a.get("digest")) for a in json.load(sys.stdin)["assets"]]'

step "tool versions on PATH"
for t in m4 bison flex; do printf '%-6s %s  ' "$t" "$(command -v $t)"; "$t" --version | head -1; done

clone_at() { # url dir sha tree
  local url="$1" dir="$2" sha="$3" tree="$4"
  step "clone $(basename "$dir") at $sha"
  [[ -d "$dir/.git" ]] || git clone --quiet --filter=blob:none "$url" "$dir"
  git -C "$dir" -c advice.detachedHead=false checkout --quiet "$sha"
  local head t
  head="$(git -C "$dir" rev-parse HEAD)"; t="$(git -C "$dir" rev-parse 'HEAD^{tree}')"
  echo "HEAD=$head tree=$t"
  [[ "$head" == "$sha" ]] || { echo "FATAL: HEAD mismatch"; exit 1; }
  [[ "$t" == "$tree" ]] || { echo "FATAL: tree differs from the laptop read copy"; exit 1; }
  echo "dirty files: $(git -C "$dir" status --porcelain | wc -l)"
  git -C "$dir" log -1 --format='commit date=%cI subject=%s'
}
clone_at https://github.com/unikraft/unikraft.git "$W/unikraft" "$UK_SHA" "$UK_TREE"
clone_at https://github.com/unikraft/lib-lwip.git "$W/lib-lwip" "$LWIP_SHA" "$LWIP_TREE"

step "done"
du -sh "$W/tools" "$W/unikraft" "$W/lib-lwip"
