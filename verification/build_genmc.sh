#!/usr/bin/env bash
# Build GenMC (the C11 stateless model checker) from source against
# the LLVM 18 the project already uses. One-time setup for
# verification/run_genmc.sh. Prints the resulting binary path.
#
#   verification/build_genmc.sh [install-dir]   # default: /tmp/genmc
set -euo pipefail

dest="${1:-/tmp/genmc}"
llvm_config="${LLVM_CONFIG:-llvm-config-18}"
# Pinned to the verified commit (GenMC v0.17.0) for reproducible
# builds + a stable CI cache key. Override with $GENMC_REF.
genmc_ref="${GENMC_REF:-29b03a66402c4453fc77901ef3be90bb55707cd4}"

command -v cmake >/dev/null 2>&1 || {
    echo "error: cmake required (pip install --user cmake, or apt-get install cmake)" >&2
    exit 1
}
command -v "$llvm_config" >/dev/null 2>&1 || {
    echo "error: $llvm_config not found (set \$LLVM_CONFIG)" >&2
    exit 1
}

if [ ! -d "$dest/.git" ]; then
    git clone https://github.com/MPI-SWS/genmc.git "$dest"
fi
( cd "$dest" && git checkout --quiet "$genmc_ref" )
mkdir -p "$dest/build"
( cd "$dest/build"
  cmake -DCMAKE_BUILD_TYPE=Release -DLLVM_CONFIG="$(command -v "$llvm_config")" ..
  make -j"$(nproc)" )

bin="$(find "$dest" -name genmc -type f -executable | head -1)"
echo "GenMC built: $bin"
echo "Run:  GENMC=$bin verification/run_genmc.sh"
