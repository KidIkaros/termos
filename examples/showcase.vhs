Output assets/termos-showcase.gif

Set Shell "bash"
Set FontSize 14
Set FontFamily "DejaVu Sans Mono"
Set Width 1200
Set Height 600
Set Theme "Molokai"
Set LoopOffset 0%

# Set a generic prompt (hides personal info)
Type "export PS1='termos $ '"
Sleep 100ms
Enter
Sleep 200ms

# Launch TermOS
Type "./target/release/termos"
Sleep 200ms
Enter
Sleep 2s

# Dismiss welcome
Space
Sleep 500ms

# Split horizontal
Ctrl+b Sleep 100ms Type "-"
Sleep 500ms

# Split vertical
Ctrl+b Sleep 100ms Type "|"
Sleep 500ms

# Focus right
Ctrl+b Sleep 100ms Type "l"
Sleep 300ms

# Focus down
Ctrl+b Sleep 100ms Type "j"
Sleep 300ms

# Zoom
Ctrl+b Sleep 100ms Type "z"
Sleep 500ms

# Unzoom
Ctrl+b Sleep 100ms Type "z"
Sleep 300ms

# Help overlay
Ctrl+b Sleep 100ms Type "?"
Sleep 1s

# Close help
Escape
Sleep 300ms

# Command palette
Ctrl+b Sleep 100ms Type "P"
Sleep 500ms

# Type "theme"
Type "theme"
Sleep 500ms

# Select
Enter
Sleep 500ms

# Navigate themes
Down Down Enter
Sleep 500ms

# Final layout pause
Sleep 2s

# Quit
Ctrl+b Sleep 100ms Type "q"
Sleep 200ms
