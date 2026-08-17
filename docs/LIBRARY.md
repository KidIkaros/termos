# Using TermOS as a Library

TermOS can be imported and used as a library in your own Rust applications. This allows you to embed a full-featured terminal window manager in your ratatui applications.

## Installation

Add TermOS to your `Cargo.toml`:

```toml
[dependencies]
termos = { git = "https://github.com/Gaurav-Gosain/tuios" }
ratatui = "0.29"
crossterm = "0.28"
```

## Quick Start

### Basic Usage

```rust
use std::io;
use termos::app::App;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen};
use crossterm::execute;

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Create a new TermOS instance with default options
    let mut app = App::new();

    // Run the main loop
    app.run(&mut terminal)?;

    Ok(())
}
```

### With Custom Options

```rust
let app = App::builder()
    .theme("dracula")
    .show_keys(true)
    .animations(false)
    .workspaces(9)
    .border_style("rounded")
    .dockbar_position("bottom")
    .scrollback_lines(20000)
    .build();
```

## Options Reference

### theme(name: &str)

Set the color theme. Available themes include "dracula", "nord", "tokyonight", and others.

```rust
App::builder().theme("dracula")
```

### show_keys(enabled: bool)

Enable the showkeys overlay to display pressed keys (useful for demos).

```rust
App::builder().show_keys(true)
```

### animations(enabled: bool)

Enable or disable window animations. When disabled, windows snap instantly.

```rust
App::builder().animations(false)
```

### ascii_only(enabled: bool)

Use ASCII characters instead of Nerd Font icons for compatibility.

```rust
App::builder().ascii_only(true)
```

### workspaces(n: usize)

Set the number of workspaces (1-9).

```rust
App::builder().workspaces(4)
```

### border_style(style: &str)

Set the window border style. Valid values:
- `"rounded"` (default)
- `"normal"`
- `"thick"`
- `"double"`
- `"hidden"`
- `"block"`
- `"ascii"`

```rust
App::builder().border_style("thick")
```

### dockbar_position(position: &str)

Set the dockbar position. Valid values:
- `"bottom"` (default)
- `"top"`
- `"hidden"`

```rust
App::builder().dockbar_position("top")
```

### hide_window_buttons(hide: bool)

Hide the minimize/maximize/close buttons in window title bars.

```rust
App::builder().hide_window_buttons(true)
```

### scrollback_lines(lines: usize)

Set the scrollback buffer size (100-1000000).

```rust
App::builder().scrollback_lines(50000)
```

### size(width: u16, height: u16)

Set the initial terminal size. Usually not needed as TermOS auto-detects.

```rust
App::builder().size(120, 40)
```

### ssh_mode(enabled: bool)

Enable SSH mode for running over SSH connections.

```rust
App::builder().ssh_mode(true)
```

### user_config(cfg: UserConfig)

Provide a custom user configuration instead of loading from file.

```rust
let mut cfg = UserConfig::default();
cfg.keybindings.leader_key = "ctrl+a";
App::builder().user_config(cfg)
```

## Web Terminal Integration

TermOS can be served through the browser using the web server feature:

```rust
use termos::network::web::WebServer;
use termos::network::web::WebConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = WebServer::new(WebConfig {
        host: "localhost".to_string(),
        port: 7681,
        ..Default::default()
    });

    server.serve().await?;
    Ok(())
}
```

## SSH Server Integration

For SSH server integration, enable the `network` feature:

```toml
[dependencies]
termos = { git = "https://github.com/Gaurav-Gosain/tuios", features = ["network"] }
```

```rust
use termos::network::ssh::SshServer;
use termos::network::ssh::SshConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = SshServer::new(SshConfig {
        address: "0.0.0.0:2222".to_string(),
        ..Default::default()
    });

    server.listen_and_serve().await?;
    Ok(())
}
```

## Configuration Access

The `Config` module provides access to configuration utilities:

```rust
use termos::config::UserConfig;

// Load user config from file
let cfg = UserConfig::load()?;

// Get default config
let cfg = UserConfig::default();

// Get config file path
let path = UserConfig::config_path()?;
```

## App Methods

The TermOS `App` provides several public methods:

### Window Management

- `add_window(title: &str)` - Create a new terminal window
- `delete_window(index: usize)` - Close window at index
- `focus_window(index: usize)` - Focus window at index
- `get_focused_window()` - Get the currently focused window

### Workspace Management

- `switch_workspace(n: usize)` - Switch to workspace n (1-9)
- `move_window_to_workspace(window_index: usize, workspace: usize)` - Move a window

### Layout

- `toggle_tiling()` - Toggle automatic tiling mode
- `tile_all_windows()` - Retile all windows

### Cleanup

- `cleanup()` - Clean up resources (call when done)

## Example: Custom Wrapper

You can wrap TermOS in your own model for additional functionality:

```rust
use termos::app::App;

struct MyApp {
    termos: App,
    // your additional state
}

impl MyApp {
    fn new() -> Self {
        Self {
            termos: App::new(),
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        // Handle your custom logic, then delegate to TermOS
        self.termos.run(terminal)
    }
}
```

## Related Documentation

- [Architecture](ARCHITECTURE.md) - Technical architecture
- [Keybindings](KEYBINDINGS.md) - Keyboard shortcuts
- [Configuration](CONFIGURATION.md) - Config file options
- [Web Terminal](WEB.md) - Browser-based access
- [Sip Library](SIP_LIBRARY.md) - Web serving library
