//! Property-based tests for config parsing — arbitrary TOML must not panic.

use proptest::prelude::*;
use termos::config::userconfig::UserConfig;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Loading a config from arbitrary string content must never panic.
    /// It may fail (return default), but must not crash.
    #[test]
    fn config_parse_never_panics(ref input in ".*") {
        let _ = UserConfig::parse_str(input);
    }

    /// Loading a config from arbitrary bytes must never panic.
    #[test]
    fn config_parse_bytes_never_panics(ref input in prop::collection::vec(any::<u8>(), 0..1024)) {
        let s = String::from_utf8_lossy(input);
        let _ = UserConfig::parse_str(&s);
    }
}
