use std::path::PathBuf;

/// Layered configuration: environment variable overrides > config file > defaults.
///
/// Kept in `amx-core` (portable) rather than `amx-store` so Linux CI can construct one without
/// a live mailbox; the default `store_path` is only ever populated on macOS (task 4's
/// constraint: no macOS path in portable code's unconditional defaults).
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `None` on a platform/build with no default store location — caller must set
    /// `AMX_STORE_PATH` or the config file's `store_path`.
    pub store_path: Option<PathBuf>,
    pub cache_dir: PathBuf,
    pub read_only: bool,
}

#[derive(Debug, Default, serde::Deserialize)]
struct FileConfig {
    store_path: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    read_only: Option<bool>,
}

impl Config {
    /// Resolves env vars, then an optional config file, then built-in defaults, in that
    /// precedence order.
    pub fn load() -> Self {
        Self::load_from(std::env::var("AMX_CONFIG_FILE").ok().map(PathBuf::from))
    }

    fn load_from(config_file_override: Option<PathBuf>) -> Self {
        let file = config_file_override
            .or_else(default_config_file)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|contents| toml::from_str::<FileConfig>(&contents).ok())
            .unwrap_or_default();

        let store_path = std::env::var("AMX_STORE_PATH")
            .ok()
            .map(PathBuf::from)
            .or(file.store_path)
            .or_else(default_store_path);

        let cache_dir = std::env::var("AMX_CACHE_DIR")
            .ok()
            .map(PathBuf::from)
            .or(file.cache_dir)
            .or_else(default_cache_dir)
            .unwrap_or_else(|| PathBuf::from(".amx-cache"));

        let read_only = std::env::var("AMX_READ_ONLY")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .or(file.read_only)
            .unwrap_or(false);

        Self {
            store_path,
            cache_dir,
            read_only,
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

fn default_config_file() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".config/apple-mail-mcp/config.toml"))
}

#[cfg(target_os = "macos")]
fn default_store_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Library/Mail/V10"))
}

#[cfg(not(target_os = "macos"))]
fn default_store_path() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn default_cache_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Library/Caches/apple-mail-mcp"))
}

#[cfg(not(target_os = "macos"))]
fn default_cache_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".cache/apple-mail-mcp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Both cases share process-wide env vars, so they run as one test — `cargo test` runs
    // tests in parallel by default, and two tests toggling the same vars would race.
    #[test]
    fn env_precedence() {
        // SAFETY: test-local env mutation; no other test in this module reads these vars.
        unsafe {
            std::env::remove_var("AMX_STORE_PATH");
            std::env::remove_var("AMX_CACHE_DIR");
            std::env::remove_var("AMX_READ_ONLY");
        }
        let config = Config::load_from(Some(PathBuf::from("/nonexistent/config.toml")));
        assert!(!config.read_only);
        assert!(config.cache_dir.ends_with("apple-mail-mcp"));

        // SAFETY: test-local env mutation; no other test in this module reads these vars.
        unsafe {
            std::env::set_var("AMX_READ_ONLY", "true");
        }
        let config = Config::load_from(Some(PathBuf::from("/nonexistent/config.toml")));
        assert!(config.read_only);
        // SAFETY: test-local env mutation; no other test in this module reads these vars.
        unsafe {
            std::env::remove_var("AMX_READ_ONLY");
        }
    }
}
