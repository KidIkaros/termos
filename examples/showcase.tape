# TermOS Showcase — focused demo for GIF recording
# Ends with Quit so automated recording exits cleanly

DisableAnimations
WindowManagementMode
Sleep 500ms

# --- Create 3 windows ---
NewWindow
Sleep 300ms
NewWindow
Sleep 300ms
NewWindow
Sleep 300ms

# --- Split panes ---
Split
Sleep 300ms

# --- Focus navigation ---
NextWindow
Sleep 200ms
NextWindow
Sleep 200ms
NextWindow
Sleep 200ms

# --- Type in terminal ---
TerminalMode
Type "echo 'TermOS v0.1.1 — Terminal Multiplexer'"
Enter
Sleep 400ms
Type "echo 'BSP Tiling + Master-Stack Layouts'"
Enter
Sleep 300ms
Type "echo '21 Built-in Themes • Config Hot-Reload'"
Enter
Sleep 300ms
WindowManagementMode

# --- Command palette ---
Ctrl+B P
Sleep 800ms
Escape
Sleep 300ms

# --- Help overlay ---
Ctrl+B ?
Sleep 1200ms
Escape
Sleep 300ms

# --- Zoom ---
Ctrl+B z
Sleep 800ms
TerminalMode
Type "echo 'Full-screen zoom'"
Enter
Sleep 500ms
WindowManagementMode
Ctrl+B z
Sleep 500ms

# --- Workspace switch ---
Alt+2
Sleep 600ms
TerminalMode
Type "echo 'Workspace 2'"
Enter
Sleep 400ms
WindowManagementMode
Alt+1
Sleep 500ms

# --- Branding shot ---
TerminalMode
Type "echo ''"
Enter
Type "echo '  ████████╗███████╗██████╗ ███╗   ███╗'"
Enter
Type "echo '  ╚══██╔══╝██╔════╝██╔══██╗████╗ ████║'"
Enter
Type "echo '     ██║   █████╗  ██████╔╝██╔████╔██║'"
Enter
Type "echo '     ██║   ██╔══╝  ██╔══██╗██║╚██╔╝██║'"
Enter
Type "echo '     ██║   ███████╗██║  ██║██║ ╚═╝ ██║'"
Enter
Type "echo '     ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝     ╚═╝'"
Enter
Type "echo ''"
Enter
Type "echo '  github.com/KidIkaros/termos'"
Enter
Sleep 1s
WindowManagementMode
Sleep 500ms
