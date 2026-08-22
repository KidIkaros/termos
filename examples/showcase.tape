# TermOS Showcase — features demo for GIF recording
# Uses only supported tape commands

DisableAnimations

# --- Opening: clean single pane ---
WindowManagementMode
Sleep 500ms

# --- Create multiple windows ---
NewWindow
Sleep 300ms
NewWindow
Sleep 300ms
NewWindow
Sleep 300ms

# --- Split panes ---
Split
Sleep 200ms

# --- Focus different windows ---
NextWindow
Sleep 200ms
NextWindow
Sleep 200ms
NextWindow
Sleep 200ms

# --- Type commands in terminal mode ---
TerminalMode
Type "echo 'TermOS v0.1.0 — Terminal Multiplexer'"
Enter
Sleep 400ms
Type "echo 'BSP Tiling + Master-Stack Layouts'"
Enter
Sleep 300ms
Type "echo '21 Built-in Themes • Config Hot-Reload'"
Enter
Sleep 300ms
WindowManagementMode

# --- Open command palette ---
Ctrl+B P
Sleep 600ms
Escape
Sleep 300ms

# --- Open help overlay ---
Ctrl+B ?
Sleep 800ms
Escape
Sleep 300ms

# --- Zoom a pane ---
Ctrl+B z
Sleep 600ms
TerminalMode
Type "echo 'Full-screen zoom — press z to exit'"
Enter
Sleep 500ms
WindowManagementMode
Ctrl+B z
Sleep 400ms

# --- Switch workspace ---
Alt+2
Sleep 500ms
TerminalMode
Type "echo 'Workspace 2 — multi-workspace support'"
Enter
Sleep 400ms
WindowManagementMode

# --- Back to workspace 1 ---
Alt+1
Sleep 500ms

# --- Close extra windows ---
CloseWindow
Sleep 200ms
CloseWindow
Sleep 200ms
CloseWindow
Sleep 400ms

# --- Final shot: branding ---
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
