# Source me (inside Lima): puts the spike-local Quint + JDK on PATH and pins QUINT_HOME / target dir.
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$INC/.tools/bin:$INC/.tools/jdk/bin:$INC/.tools/protoc/bin:$PATH"
export PROTOC="$INC/.tools/protoc/bin/protoc"
export JAVA_HOME="$INC/.tools/jdk"
# Apalache's launcher defaults to a 4 GiB JVM heap. The 4-voter reconfiguration model exhausts that
# heap during bounded preprocessing; Lima provides 16 GiB with roughly 13 GiB available at rest, so
# reserve 8 GiB for the JVM and leave the other half for Z3, the OS, and Quint.
export JVM_ARGS="${JVM_ARGS:--Xmx8g}"
export QUINT_HOME="$INC/.tools/quint-home"
export CARGO_TARGET_DIR="$INC/target"
