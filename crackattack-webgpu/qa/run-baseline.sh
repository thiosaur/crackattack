#!/bin/sh
# Reference capture campaign.
# Regenerates the captures in qa/current/ (emptied first: names are deterministic, so an
# existing file would make the capture fail), then compares them to the baseline BY NAME.
# Only DETERMINISTIC captures are in the manifest. The autoplay scripts also write captures
# that are NOT compared (shader-vs-ia-*, round3-visible, anim dying-awaking): two runs of
# the same binary give different images, so a hash on them would be a coin flip. They are
# there to be looked at, not asserted.
# Rotation and archiving are the policy of THIS launcher, not of the game.
set -e
cd "$(dirname "$0")/.."    # crackattack-webgpu/
OUT="qa/current"
rm -rf "$OUT"
run() { cargo run -q -p crackattack-webgpu -- "$@" --set session:qa.capture_dir="$OUT"; }
run --preset solo --set session:game.seed=42 --script qa/scripts/shader-solo.mini
run --preset solo --set session:game.seed=42 --script qa/scripts/shader-solo-panic.mini
run --preset vs-computer hard --set session:game.seed=42 --set session:autoplay=true --script qa/scripts/shader-vs-ia.mini
run --preset vs-computer hard --set session:game.seed=42 --set session:autoplay=true --script qa/scripts/shader-vs-ia-shatter.mini
run --preset vs-computer hard --set session:game.seed=42 --set session:autoplay=true --script qa/scripts/shader-vs-ia-anim.mini
run --preset vs-computer hard --set session:game.seed=42 --set session:autoplay=true --script qa/scripts/shader-vs-ia-round3.mini
# shasum (Perl) is missing on many Linux distros: fall back to sha256sum.
if command -v shasum >/dev/null 2>&1; then SHA="shasum -a 256"; else SHA="sha256sum"; fi
( cd "$OUT" && $SHA -c ../BASELINE_SHA256.txt )
echo "Baseline: the 9 deterministic captures match. Not compared (autoplay, look at them by eye): $OUT/shader-vs-ia-*.png"
