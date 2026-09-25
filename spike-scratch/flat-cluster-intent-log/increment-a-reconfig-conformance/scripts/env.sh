# Source me (inside Lima): puts the spike-local Quint + JDK on PATH and pins QUINT_HOME / target dir.
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$INC/.tools/bin:$INC/.tools/jdk/bin:$INC/.tools/protoc/bin:$PATH"
export PROTOC="$INC/.tools/protoc/bin/protoc"
export JAVA_HOME="$INC/.tools/jdk"
export QUINT_HOME="$INC/.tools/quint-home"
export CARGO_TARGET_DIR="$INC/target"
