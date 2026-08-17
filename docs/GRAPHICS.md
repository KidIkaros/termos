# Graphics Passthrough

TermOS is a passthrough-first terminal multiplexer for images: it does not
rasterize images into text cells. Instead it forwards Kitty graphics
protocol and Sixel sequences to the host terminal, rewriting image IDs
and placement coordinates so images follow their panes.

## Supported Protocols

| Protocol | Terminals | Detection |
|----------|-----------|-----------|
| Kitty APC | kitty, Ghostty, WezTerm | `TERM_PROGRAM`, `KITTY_WINDOW_ID`, `GHOSTTY_RESOURCES_DIR` |
| Sixel | WezTerm, Alacritty, xterm (vt340), foot, mlterm | `TERM` contains `vt340`/`mlterm`/`foot` |

## How It Works

1. **Capability probing** (`graphics/capability.rs`): At startup, TermOS
   reads `TERM_PROGRAM`, `TERM`, and terminal-specific env vars to
   determine which graphics protocols the host supports.

2. **Sequence collection** (`vt/emulator.rs`): The VT parser collects
   Kitty APC sequences (G-prefixed) and Sixel DCS sequences (q-terminated)
   into per-window pending queues.

3. **Forwarding** (`graphics/kitty.rs`, `graphics/sixel.rs`): Once per
   render tick, `Os::flush_graphics` drains the queues and forwards each
   sequence to the host terminal:
   - Kitty: rewrites `i=<id>` to a per-window host ID, offsets `x=`/`y=`
     by the pane's absolute screen position
   - Sixel: positions the cursor at the pane origin with CUP, then
     forwards the DCS stream

4. **Placement tracking** (`graphics/placement.rs`): Each placed image is
   recorded with its (window, guest_image_id, host_image_id, position).
   When panes move or resize, `refresh_placements` re-emits `a=p` (place)
   commands at the new positions.

## Limitations

- No image rasterization: if the host terminal doesn't support Kitty or
  Sixel, images are silently dropped.
- Sixel has no per-image delete; clearing uses a full-screen erase.
- The web terminal (xterm.js) doesn't support Kitty/Sixel natively; a
  custom overlay would be needed (see the Go project's
  `xterm-kitty-overlay.js`).

## Multiplexer Nesting

When TermOS runs inside tmux or screen, graphics sequences may be filtered
by the outer multiplexer. The capability probe detects this
(`inside_multiplexer`) but cannot work around it — the outer multiplexer
must be configured to pass through APC/DCS sequences.
