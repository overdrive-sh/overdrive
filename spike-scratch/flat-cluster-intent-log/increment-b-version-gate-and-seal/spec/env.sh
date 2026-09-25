# Source inside the Lima VM. Tooling lives outside git, in the VM's /tmp (see ../.gitignore note in
# findings): Node 22.19.0 (linux-arm64), Temurin JDK 21.0.12.1, @informalsystems/quint 0.32.0, and
# the Apalache 0.56.1 distribution Quint downloads into QUINT_HOME on first `quint verify`.
T=/tmp/fcil-incb-tools
export PATH="$T/node-v22.19.0-linux-arm64/bin:$T/jdk-21.0.12.1+1/bin:$T/quint/node_modules/.bin:$PATH"
export JAVA_HOME="$T/jdk-21.0.12.1+1"
export QUINT_HOME="$T/quint-home"
# A private Apalache server port so this increment never attaches to a sibling agent's server.
export APALACHE_ENDPOINT="localhost:18922"
SPEC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export SPEC_DIR
export EVIDENCE_DIR="$SPEC_DIR/../evidence"
