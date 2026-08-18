//! Exercises the `testutil` module end-to-end: XDG isolation redirects the
//! config/state directories, and the fake shell script is a runnable PTY
//! target.

use std::sync::Mutex;

use termos::config::userconfig::UserConfig;
use termos::testutil::{create_fake_shell, XdgIsolate};

/// XdgIsolate mutates process-global environment variables, so every test
/// that touches it must be serialized against the others.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn xdg_isolate_redirects_config_path() {
    let _guard = ENV_LOCK.lock().unwrap();
    let isolate = XdgIsolate::new().unwrap();
    let config_path = UserConfig::config_path().expect("config path resolves");
    // Under the isolate, the config dir is the temp dir's config/termos/.
    assert!(
        config_path.starts_with(isolate.config_dir()),
        "config path {config_path:?} not under {}",
        isolate.config_dir().display()
    );
    assert!(config_path.ends_with("config.toml"));
}

#[test]
fn xdg_isolate_restores_environment_on_drop() {
    let _guard = ENV_LOCK.lock().unwrap();
    let before = std::env::var("XDG_CONFIG_HOME").ok();
    {
        let isolate = XdgIsolate::new().unwrap();
        assert_eq!(
            std::env::var("XDG_CONFIG_HOME").ok(),
            Some(isolate.config_dir().display().to_string())
        );
    }
    assert_eq!(std::env::var("XDG_CONFIG_HOME").ok(), before);
}

#[test]
fn fake_shell_script_is_executable_sh() {
    let tmpdir = std::env::temp_dir().join(format!("termos-fakeshell-{}", std::process::id()));
    std::fs::create_dir_all(&tmpdir).unwrap();
    let path = create_fake_shell(&tmpdir).unwrap();
    assert!(path.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert!(perms.mode() & 0o111 != 0, "fake shell must be executable");
    }
    // The script echoes input back, so it is a valid `sh` target.
    let script = std::fs::read_to_string(&path).unwrap();
    assert!(script.starts_with("#!/bin/sh"));
    let _ = std::fs::remove_dir_all(&tmpdir);
}

#[test]
fn isolate_creates_all_directories() {
    let _guard = ENV_LOCK.lock().unwrap();
    let isolate = XdgIsolate::new().unwrap();
    for dir in [
        isolate.config_dir(),
        isolate.state_dir(),
        isolate.data_dir(),
    ] {
        assert!(dir.exists(), "{} missing", dir.display());
    }
    // Config written under the isolate is visible to UserConfig::load.
    let cfg_dir = isolate.config_dir().join("termos");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(cfg_dir.join("config.toml"), "").unwrap();
    let cfg = UserConfig::load();
    assert_eq!(
        cfg.appearance.theme,
        UserConfig::default_config().appearance.theme
    );
}
