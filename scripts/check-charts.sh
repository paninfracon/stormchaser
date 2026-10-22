#!/bin/bash
# Lints and renders the Helm charts, and checks that the subchart archives the umbrella
# chart vendors (deploy/charts/stormchaser/charts/*.tgz) match their sources. An archive
# left stale deploys what the source no longer says, and nothing else would notice.
set -euo pipefail

CHARTS=deploy/charts
OURS=(stormchaser-orchestration stormchaser-runner-docker stormchaser-runner-k8s stormchaser-telemetry)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

for chart in "${OURS[@]}" stormchaser; do
  helm lint --quiet "$CHARTS/$chart"
done

# Render with the values a real install must give
helm template check "$CHARTS/stormchaser" > /dev/null
helm template check "$CHARTS/stormchaser-orchestration" \
  --set global.database.url=postgres://user:secret@db:5432/stormchaser > /dev/null

stale=0
for chart in "${OURS[@]}"; do
  version=$(awk '/^version:/ {print $2}' "$CHARTS/$chart/Chart.yaml" | tr -d '"')
  vendored="$CHARTS/stormchaser/charts/$chart-$version.tgz"
  if [ ! -f "$vendored" ]; then
    echo "missing: $vendored" >&2
    stale=1
    continue
  fi
  mkdir -p "$WORK/source" "$WORK/vendored/$chart"
  # Packaged the same way, so both sides are normalised alike
  helm package "$CHARTS/$chart" -d "$WORK/source" > /dev/null
  rm -rf "$WORK/fresh" && mkdir -p "$WORK/fresh"
  tar -xzf "$WORK/source/$chart-$version.tgz" -C "$WORK/fresh"
  tar -xzf "$vendored" -C "$WORK/vendored/$chart"
  if ! diff -r "$WORK/fresh/$chart" "$WORK/vendored/$chart/$chart" > "$WORK/diff"; then
    echo "stale: $vendored does not match $CHARTS/$chart:" >&2
    head -40 "$WORK/diff" >&2
    stale=1
  fi
done
if [ "$stale" -ne 0 ]; then
  echo "Repackage with: helm package $CHARTS/<chart> -d $CHARTS/stormchaser/charts" >&2
  exit 1
fi
echo "Charts lint, render, and the vendored archives match their sources."
