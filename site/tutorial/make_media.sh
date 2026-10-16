#!/usr/bin/env bash
# Makes the pictures of the tutorial page: the sample episode (macOS voices), the keyframes from the
# real interface (crates/ui-egui/tests/site_media.rs), the GIFs and the guest page on a phone.
#
#   site/tutorial/make_media.sh WORK_DIR [OUT_DIR]     OUT_DIR defaults to site/media/tutorial
#   TALKCRAFT_SCENE=recording site/tutorial/make_media.sh WORK_DIR    one scene only
#
# The interface runs with WORK_DIR/home as its home folder: it reads and writes none of your own
# TalkCraft settings, and a stand-in speech model there makes it show its normal buttons.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
WORK="$(mkdir -p "$1" && cd "$1" && pwd)"
OUT="${2:-$ROOT/site/media/tutorial}"

python3 "$HERE/make_audio.py" "$WORK"
models="$WORK/home/models"
mkdir -p "$models"
: > "$models/ggml-large-v3-turbo-q5_0.bin"

# Cargo and rustup keep their own folders; only the app sees the other home.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
(cd "$ROOT" && HOME="$WORK/home" TALKCRAFT_MODELS="$models" TALKCRAFT_MEDIA="$WORK" \
  cargo test -p soundcraft-ui-egui --test site_media -- --ignored --nocapture)

python3 "$HERE/make_gifs.py" "$WORK" "$OUT"
python3 "$HERE/guest_shot.py" "$OUT"
