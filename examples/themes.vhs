Output assets/termos-themes.gif

Set Shell "bash"
Set FontSize 14
Set FontFamily "DejaVu Sans Mono"
Set Width 1200
Set Height 600
Set LoopOffset 0%

# Set a generic prompt
Type "export PS1='termos $ '"
Sleep 100ms
Enter
Sleep 200ms

# Launch TermOS with a colorful theme
Type "./target/release/termos --theme dracula"
Sleep 200ms
Enter
Sleep 2s

# Dismiss welcome
Space
Sleep 500ms

# Create a split layout to show off the theme
Ctrl+b Sleep 100ms Type "-"
Sleep 400ms

# Switch to Catppuccin via command palette
Ctrl+b Sleep 100ms Type "P"
Sleep 400ms
Type "catppuccin"
Sleep 300ms
Enter
Sleep 1s

# Switch to Gruvbox
Ctrl+b Sleep 100ms Type "P"
Sleep 400ms
Type "gruvbox"
Sleep 300ms
Enter
Sleep 1s

# Switch to Nord
Ctrl+b Sleep 100ms Type "P"
Sleep 400ms
Type "nord"
Sleep 300ms
Enter
Sleep 1s

# Switch to Tokyo Night
Ctrl+b Sleep 100ms Type "P"
Sleep 400ms
Type "tokyo"
Sleep 300ms
Enter
Sleep 1s

# Switch to Solarized
Ctrl+b Sleep 100ms Type "P"
Sleep 400ms
Type "solarized"
Sleep 300ms
Enter
Sleep 1s

# Switch back to Dracula for the final shot
Ctrl+b Sleep 100ms Type "P"
Sleep 400ms
Type "dracula"
Sleep 300ms
Enter
Sleep 1s

# Final pause
Sleep 1s

# Quit
Ctrl+b Sleep 100ms Type "q"
Sleep 200ms
