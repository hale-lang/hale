#!/usr/bin/env bash
# Build GenMC (the C11 stateless model checker) from source against
# the LLVM 18 the project already uses. One-time setup for
# verification/run_genmc.sh. Prints the resulting binary path.
#
#   verification/build_genmc.sh [install-dir]   # default: /tmp/genmc
set -euo pipefail

dest="${1:-/tmp/genmc}"
llvm_config="${LLVM_CONFIG:-llvm-config-18}"

command -v cmake >/dev/null 2>&1 || {
    echo "error: cmake required (pip install --user cmake, or apt-get install cmake)" >&2
    exit 1
}
command -v "$llvm_config" >/dev/null 2>&1 || {
    echo "error: $llvm_config not found (set \$LLVM_CONFIG)" >&2
    exit 1
}

if [ ! -d "$dest" ]; then
    git clone --depth 1 https://github.com/MPI-SWS/genmc.git "$dest"
fi
mkdir -p "$dest/build"
( cd "$dest/build"
  cmake -DCMAKE_BUILD_TYPE=Release -DLLVM_CONFIG="$(command -v "$llvm_config")" ..
  make -j"$(nproc)" )

bin="$(find "$dest" -name genmc -type f -executable | head -1)"
echo "GenMC built: $bin"
echo "Run:  GENMC=$bin verification/run_genmc.sh"
