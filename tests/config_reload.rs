//! Hot-reload watcher integration test.
//!
//! Mirrors the live dogfood: the TUI's config file is rewritten while the
//! session is running, and the debounced watcher must deliver the new config
//! (theme + leader swap), skip broken edits, and recover afterwards.

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use termos::app::msg::Msg;
use termos::app::Os;
use termos::config::userconfig::UserConfig;

/// Rewrite the config file with the given theme and leader key.
fn write_config(path: &std::path::Path, theme: &str, leader: &str) {
    std::fs::write(
        path,
        format!(
            "[appearance]\ntheme = \"{theme}\"\nwhich_key_position = \"bottom-right\"\n\n\
             [keybindings]\nleader_key = \"{leader}\"\n"
        ),
    )
    .expect("write config");
}

#[test]
fn hot_reload_rewrites_config_mid_session() {
    let dir = std::env::temp_dir().join(format!("termos-hr-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");

    // Initial config is in place before the watcher starts, like a session
    // that has already booted with solarized-light / ctrl-b.
    write_config(&path, "solarized-light", "ctrl-b");

    let rx = UserConfig::watch(path.clone()).expect("watcher starts");

    // Mid-session rewrite: swap the theme and the leader key.
    write_config(&path, "dracula", "ctrl-a");
    let cfg = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reload delivered within 5s");
    assert_eq!(cfg.appearance.theme, "dracula");
    assert_eq!(cfg.keybindings.leader_key, "ctrl-a");

    // Drive the same message the event loop uses and assert the live swap:
    // the resolved theme and the keybinding both change without a restart.
    let mut os = Os::new(UserConfig::default_config());
    os.update(Msg::ConfigReloaded(Box::new(cfg)));
    let theme = os.theme.as_ref().expect("reload resolves the theme");
    assert_eq!(theme.name, "dracula");
    assert_eq!(os.config.keybindings.leader_key, "ctrl-a");

    // A broken edit must NOT be delivered: the watcher keeps the last good
    // config instead of resetting the session to defaults.
    std::fs::write(&path, "this is [ not valid toml ===\n").unwrap();
    match rx.recv_timeout(Duration::from_millis(800)) {
        Err(RecvTimeoutError::Timeout) => {} // expected: nothing sent
        Err(RecvTimeoutError::Disconnected) => panic!("watcher died"),
        Ok(_) => panic!("a broken config must not be delivered"),
    }

    // The watcher recovers: the next valid rewrite is delivered and applied.
    write_config(&path, "gruvbox-dark", "ctrl-b");
    let cfg2 = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("recovery reload delivered within 5s");
    assert_eq!(cfg2.appearance.theme, "gruvbox-dark");
    assert_eq!(cfg2.keybindings.leader_key, "ctrl-b");

    let mut os = Os::new(UserConfig::default_config());
    os.update(Msg::ConfigReloaded(Box::new(cfg2)));
    let theme = os.theme.as_ref().expect("recovery reload resolves the theme");
    assert_eq!(theme.name, "gruvbox-dark");
    assert_eq!(os.config.keybindings.leader_key, "ctrl-b");

    std::fs::remove_dir_all(&dir).ok();
}
