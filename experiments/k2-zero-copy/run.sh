#!/usr/bin/env bash
# Form K2 (2026-05-20) — zero-copy vs memcpy bus boundary
# microbench runner.
#
# Builds the bench and runs it. See bench.c for what the three
# paths measure and how the medians are computed.

set -euo pipefail

cd "$(dirname "$0")"

gcc -O2 -Wall -Wextra -o bench bench.c
./bench
