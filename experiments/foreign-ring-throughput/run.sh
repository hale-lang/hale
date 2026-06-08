#!/usr/bin/env bash
# Foreign-ring throughput microbench runner (Proposal B, 2026-06-08).
# Builds the bench + the shm-ring runtime and runs it. See bench.c for
# what each path measures and README.md for the findings + decision.

set -euo pipefail

cd "$(dirname "$0")"
RUNTIME=../../crates/hale-codegen/runtime/lotus_shm_ring.c

clang -O2 -Wall -Wextra -o bench bench.c "$RUNTIME" -lrt -lpthread
./bench
