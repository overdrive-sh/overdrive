#!/usr/bin/env bash
# SPIKE tooling bootstrap (run INSIDE the Lima VM). Everything lands under the gitignored
# `.tools/` directory of this increment — nothing is installed system-wide.
#
#   .tools/bin/quint      Quint v0.32.0 standalone binary (linux-arm64 or linux-amd64)
#   .tools/jdk/           Temurin JDK 21 (Apalache needs a JVM >= 17)
#   .tools/quint-home/    QUINT_HOME: Quint downloads the Apalache distribution here on first verify
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
TOOLS="$HERE/.tools"
mkdir -p "$TOOLS/bin" "$TOOLS/quint-home"

QUINT_VERSION=v0.32.0
case "$(uname -m)" in
  aarch64 | arm64) QARCH=arm64; JARCH=aarch64 ;;
  x86_64) QARCH=amd64; JARCH=x64 ;;
  *) echo "unsupported arch $(uname -m)"; exit 1 ;;
esac

if [ ! -x "$TOOLS/bin/quint" ]; then
  curl -fsSL -o "$TOOLS/bin/quint" \
    "https://github.com/quint-co/quint/releases/download/${QUINT_VERSION}/quint-linux-${QARCH}"
  chmod +x "$TOOLS/bin/quint"
fi

if [ ! -x "$TOOLS/jdk/bin/java" ]; then
  curl -fsSL -o "$TOOLS/jdk.tar.gz" \
    "https://api.adoptium.net/v3/binary/latest/21/ga/linux/${JARCH}/jdk/hotspot/normal/eclipse"
  mkdir -p "$TOOLS/jdk"
  # `legal/` holds only license texts stored as hard links, which the virtiofs mount refuses — skip it.
  tar -xzf "$TOOLS/jdk.tar.gz" -C "$TOOLS/jdk" --strip-components=1 --exclude='*/legal'

  rm -f "$TOOLS/jdk.tar.gz"
fi

# protoc: viewstamp-proto's build.rs (buffa-build) shells out to `protoc` for the wire schema.
PROTOC_VERSION=36.2
case "$(uname -m)" in aarch64 | arm64) PARCH=aarch_64 ;; *) PARCH=x86_64 ;; esac
if [ ! -x "$TOOLS/protoc/bin/protoc" ]; then
  curl -fsSL -o "$TOOLS/protoc.zip" \
    "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${PARCH}.zip"
  mkdir -p "$TOOLS/protoc"
  (cd "$TOOLS/protoc" && python3 -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall('.')" "$TOOLS/protoc.zip")
  chmod +x "$TOOLS/protoc/bin/protoc"
  rm -f "$TOOLS/protoc.zip"
fi

echo "quint: $("$TOOLS/bin/quint" --version)"
echo "protoc: $("$TOOLS/protoc/bin/protoc" --version)"
"$TOOLS/jdk/bin/java" -version 2>&1 | head -1
