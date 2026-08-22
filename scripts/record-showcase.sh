#!/usr/bin/env bash
# Record the TermOS showcase tape as an asciinema cast, then convert to GIF.
# Usage: scripts/record-showcase.sh

set -euo pipefail

CAST_FILE="termos-showcase.cast"
GIF_FILE="termos-showcase.gif"
RECORDING_SCRIPT="examples/showcase.tape"
TERMOS_BIN="target/release/termos"

# Ensure the binary exists
if [ ! -f "$TERMOS_BIN" ]; then
    echo "Building termos..."
    cargo build --release
fi

# Ensure tmux is available
if ! command -v tmux &>/dev/null; then
    echo "tmux is required for recording"
    exit 1
fi

# Kill any existing recording session
tmux kill-session -t termos-record 2>/dev/null || true

# Start asciinema recording
echo "Starting asciinema recording..."
asciinema rec --command "tmux new-session -s termos-record -x 120 -y 30" \
    --title "TermOS Showcase" \
    --cols 120 \
    --rows 30 \
    "$CAST_FILE" &
REC_PID=$!

# Wait for tmux session to start
sleep 2

# Send the tape playback command to tmux
echo "Playing showcase tape..."
tmux send-keys -t termos-record "$PWD/$TERMOS_BIN tape play $RECORDING_SCRIPT" Enter

# Wait for the tape to finish (showcase is ~30 seconds)
echo "Waiting for tape playback to complete..."
sleep 35

# Stop the recording
echo "Stopping recording..."
kill "$REC_PID" 2>/dev/null || true
wait "$REC_PID" 2>/dev/null || true

# Clean up tmux session
tmux kill-session -t termos-record 2>/dev/null || true

# Convert to GIF
if command -v agg &>/dev/null; then
    echo "Converting to GIF..."
    agg --font-size 14 --line-height 1.2 "$CAST_FILE" "$GIF_FILE"
    echo "Done! GIF saved to $GIF_FILE"
    ls -lh "$GIF_FILE"
else
    echo "agg not installed. Cast saved to $CAST_FILE"
    echo "Install agg: cargo install --git https://github.com/asciinema/agg"
    echo "Then run: agg $CAST_FILE $GIF_FILE"
fi
