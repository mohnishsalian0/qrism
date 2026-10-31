#!/usr/bin/env bash
# Dumps HiQ's per-module classifier inputs and decisions for the parity pass. Run from the
# qrism root; needs the HiQ bench built (`bench/build.sh` in hiq-decoder).
#
#   benches/color/hiq_parity/dump.sh
#   cargo bench --features benchmark --bench color -- hiq_parity
set -euo pipefail

HIQ="${HIQ:-$HOME/Projects/hiq/HiQ/hiq-decoder}"
JAVA_HOME="${JAVA_HOME:-/opt/homebrew/opt/openjdk@21}"
HERE="benches/color/hiq_parity"
DATA="benches/dataset/high_capacity/hiq"
CP="$HIQ/bench/out:$HIQ/bench/lib/jama-1.0.3.jar:$HIQ/bench/lib/commons-math3-3.6.1.jar"

# Five photos from HiQ's test split, one per version, each from a different capture folder.
PHOTOS=(01/402.JPG 03/1149.JPG 05/2001.JPG 07/3004.JPG 08/4097.JPG)

mkdir -p "$HERE/build"
"$JAVA_HOME/bin/javac" -nowarn -cp "$CP" -d "$HERE/build" "$HERE/HiqParityDump.java"
"$JAVA_HOME/bin/java" -cp "$HERE/build:$CP" bench.HiqParityDump --root "$HIQ" --out "$HERE" \
  "${PHOTOS[@]/#/$DATA/}"
