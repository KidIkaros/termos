#!/usr/bin/env bash
# Clean TermOS showcase recording using tmux capture-pane
set -euo pipefail

CAST="termos-showcase.cast"
GIF="assets/termos-showcase.gif"
S="tr"
ROWS=30
COLS=120

rm -f "$CAST" "$GIF"
mkdir -p assets

# Start tmux with exact dimensions
tmux kill-session -t "$S" 2>/dev/null || true
tmux new-session -d -s "$S" -x "$COLS" -y "$ROWS"

# Helper: capture pane and emit asciinema event
T=0
PREV=""
frame() {
    local content ts esc
    content=$(tmux capture-pane -t "$S" -p -S -"$ROWS" 2>/dev/null)
    if [[ "$content" != "$PREV" ]]; then
        ts=$(awk "BEGIN{printf \"%.6f\",$T}")
        esc=$(printf '%s' "$content" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))')
        printf '[%s,"o",%s]\n' "$ts" "$esc" >> "$CAST"
        PREV="$content"
    fi
}

# Header
printf '{"version":2,"width":%d,"height":%d,"timestamp":%d,"env":{"SHELL":"/bin/bash","TERM":"xterm-256color"}}\n' \
    "$COLS" "$ROWS" "$(date +%s)" > "$CAST"

echo "🚀 Launching TermOS..."
tmux send-keys -t "$S" "./target/release/termos" Enter
sleep 3.5
T=3.5
frame

echo "👋 Dismiss welcome..."
tmux send-keys -t "$S" Space
sleep 1.5
T=5.0
frame

echo "🔪 Split horizontal..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" -
sleep 1.5
T=7.0
frame

echo "🔪 Split vertical..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" |
sleep 1.5
T=9.0
frame

echo "➡️  Focus right..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" l
sleep 0.8
T=10.0
frame

echo "⬇️  Focus down..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" j
sleep 0.8
T=11.0
frame

echo "🔍 Zoom pane..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" z
sleep 1.5
T=13.0
frame

echo "🔍 Unzoom..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" z
sleep 1
T=14.5
frame

echo "❓ Help overlay..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" ?
sleep 2
T=17.0
frame

echo "❌ Close help..."
tmux send-keys -t "$S" Escape
sleep 0.8
T=18.0
frame

echo "🎨 Command palette..."
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" P
sleep 1
T=19.5
frame

echo "📝 Type 'theme'..."
for c in t h e m e; do
    tmux send-keys -t "$S" "$c"
    sleep 0.2
done
sleep 0.5
T=21.0
frame

echo "✅ Select theme..."
tmux send-keys -t "$S" Enter
sleep 1.5
T=23.0
frame

echo "🏢 Switch workspace 2..."
tmux send-keys -t "$S" M-2
sleep 1.5
T=25.0
frame

echo "🏢 Switch workspace 1..."
tmux send-keys -t "$S" M-1
sleep 1.5
T=27.0
frame

echo "⏸️  Final layout..."
sleep 2
T=29.0
frame

# Quit
tmux send-keys -t "$S" C-b
sleep 0.3
tmux send-keys -t "$S" q
sleep 0.5

# Cleanup
tmux kill-session -t "$S" 2>/dev/null || true

echo "📊 Frames captured: $(grep -c '"o"' "$CAST")"
echo "📊 Cast file size: $(wc -c < "$CAST") bytes"

# Convert to GIF
echo "🖼️  Converting to GIF..."
agg \
  --font-family "DejaVu Sans Mono" \
  --font-size 14 \
  --line-height 18 \
  --font-antialiasing off \
  --theme monokai \
  --speed 1.5 \
  --idle-time-limit 0.2 \
  --select 0..100% \
  "$CAST" "$GIF" 2>&1

echo "✅ Done!"
ls -la "$GIF"
file "$GIF"
