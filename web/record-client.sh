#!/usr/bin/env bash
# Capture a matching-engine session into a browser scenario file.
#
# Run the engine with the client mix you want to show off (e.g. only the
# market-maker client active), then:
#
#   ./web/record-client.sh <id> <symbol> [endpoint]
#   ./web/record-client.sh market-maker AAPL
#   ./web/record-client.sh hft AAPL http://localhost:50051
#
# The adapter renders live while recording; press `q` to stop — it flushes the
# file on quit. The file lands at web/scenarios/<id>.txt; add a matching entry to
# SCENARIOS in web/main.js (the recorded prices/sizes are replay-param agnostic,
# so only the browser entry's interval/tick affect how it buckets).
set -euo pipefail

id="${1:?usage: record-client.sh <id> <symbol> [endpoint]}"
symbol="${2:?usage: record-client.sh <id> <symbol> [endpoint]}"
endpoint="${3:-http://localhost:50051}"

here="$(cd "$(dirname "$0")" && pwd)"
out="$here/scenarios/${id}.txt"
mkdir -p "$here/scenarios"

echo "recording '$symbol' from $endpoint → $out"
echo "press q in the chart when you have enough; the file flushes on quit."
cd "$here/.."
cargo run -p examples-matching-engine -- \
  --endpoint "$endpoint" --symbol "$symbol" --heatmap --record "$out"

echo
echo "saved $out ($(wc -l < "$out") events)."
echo "add an entry to SCENARIOS in web/main.js with id '$id' to show it."
