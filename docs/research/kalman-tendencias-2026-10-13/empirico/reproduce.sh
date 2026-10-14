#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
mkdir -p data results
archive=../../../../preset/tools/research/kalman-intraday-2026-10-08/history.tar.gz
if [[ ! -f data/UsaTec_M15.csv || ! -f data/UsaInd_M15.csv ]]; then
  tar -xzf "$archive" -C data UsaTec_M15.csv UsaInd_M15.csv
fi
if [[ -z "${KALMAN_DATA:-}" ]]; then
  tar -xzf "$archive" -C data UsaTec_M5.csv UsaTec_H1.csv UsaInd_M5.csv UsaInd_H1.csv
  export KALMAN_DATA="$PWD/data"
fi
cargo rustc --offline --release --bin diagnostics -- --cfg test
./target/release/diagnostics > results/run.log 2>&1
python3 analyze.py
