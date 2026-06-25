/// Application path resolution with CLI overrides.
///
/// All directory paths (config, cache, data, logs) are resolved once at
/// startup via `init_from_args()` and stored in a `OnceLock<AppPaths>`.
/// Default locations follow OS conventions (`dirs` crate); CLI flags
/// `--base-dir` and `--config-dir`/`--cache-dir`/`--data-dir`/`--log-dir`
/// can override them.
///
/// Bundle detection (`is_bundled`, `assets_dir`) is still used for Bevy's
/// AssetPlugin, which needs to find `assets/` relative to the executable.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Resolved application directory paths, initialized once at startup.
#[derive(Debug)]
pub struct AppPaths {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
    pub log: PathBuf,
}

static PATHS: OnceLock<AppPaths> = OnceLock::new();

/// Initialize application paths. Must be called once at startup before
/// any path functions are used. Prefer `init_from_args()` which parses
/// CLI arguments and calls this.
fn init(paths: AppPaths) {
    PATHS.set(paths).expect("paths::init called more than once");
}

fn get_paths() -> &'static AppPaths {
    PATHS
        .get()
        .expect("paths::init was not called before accessing paths")
}

/// Build default OS-standard paths for the current platform.
pub fn os_defaults() -> AppPaths {
    AppPaths {
        config: dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("airjedi"),
        cache: dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("airjedi"),
        data: dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from(".local/share"))
            .join("airjedi")
            .join("data"),
        log: if cfg!(target_os = "macos") {
            dirs::home_dir()
                .map(|h| h.join("Library/Logs/airjedi"))
                .unwrap_or_else(|| std::env::temp_dir().join("airjedi"))
        } else {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from(".local/share"))
                .join("airjedi")
                .join("logs")
        },
    }
}

const HELP_TEXT: &str = "\
AirJedi - Aircraft Map Tracker

USAGE: airjedi [OPTIONS]

OPTIONS:
    --base-dir <PATH>    Base directory for all app data (config/cache/data/logs)
    --config-dir <PATH>  Configuration directory (default: OS standard)
    --cache-dir <PATH>   Tile cache directory (default: OS standard)
    --data-dir <PATH>    Data directory for recordings/exports (default: OS standard)
    --log-dir <PATH>     Log file directory (default: OS standard)
    -h, --help           Print this help message
";

/// Parse command-line arguments and initialize application paths.
///
/// Call this once at the top of `main()` before `App::new()`.
/// Exits the process on `--help` or invalid arguments.
pub fn init_from_args() {
    let paths = parse_args(std::env::args().skip(1).collect());
    match paths {
        Ok(p) => {
            eprintln!("AirJedi paths:");
            eprintln!("  config: {}", p.config.display());
            eprintln!("  cache:  {}", p.cache.display());
            eprintln!("  data:   {}", p.data.display());
            eprintln!("  logs:   {}", p.log.display());
            init(p);
        }
        Err(e) => {
            eprintln!("Error: {e}\n");
            eprint!("{HELP_TEXT}");
            std::process::exit(1);
        }
    }
}

/// Parse arguments into `AppPaths`. Separated from `init_from_args` for testing.
fn parse_args(args: Vec<String>) -> Result<AppPaths, String> {
    let mut base_dir: Option<PathBuf> = None;
    let mut config_override: Option<PathBuf> = None;
    let mut cache_override: Option<PathBuf> = None;
    let mut data_override: Option<PathBuf> = None;
    let mut log_override: Option<PathBuf> = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP_TEXT}");
                std::process::exit(0);
            }
            "--base-dir" => {
                base_dir = Some(PathBuf::from(
                    iter.next().ok_or("--base-dir requires a path argument")?,
                ));
            }
            "--config-dir" => {
                config_override = Some(PathBuf::from(
                    iter.next().ok_or("--config-dir requires a path argument")?,
                ));
            }
            "--cache-dir" => {
                cache_override = Some(PathBuf::from(
                    iter.next().ok_or("--cache-dir requires a path argument")?,
                ));
            }
            "--data-dir" => {
                data_override = Some(PathBuf::from(
                    iter.next().ok_or("--data-dir requires a path argument")?,
                ));
            }
            "--log-dir" => {
                log_override = Some(PathBuf::from(
                    iter.next().ok_or("--log-dir requires a path argument")?,
                ));
            }
            other => {
                return Err(format!("Unknown argument: {other}"));
            }
        }
    }

    let defaults = os_defaults();

    Ok(AppPaths {
        config: config_override
            .or_else(|| base_dir.as_ref().map(|b| b.join("config")))
            .unwrap_or(defaults.config),
        cache: cache_override
            .or_else(|| base_dir.as_ref().map(|b| b.join("cache")))
            .unwrap_or(defaults.cache),
        data: data_override
            .or_else(|| base_dir.as_ref().map(|b| b.join("data")))
            .unwrap_or(defaults.data),
        log: log_override
            .or_else(|| base_dir.as_ref().map(|b| b.join("logs")))
            .unwrap_or(defaults.log),
    })
}

/// Returns `true` when the running binary lives inside a macOS `.app` bundle
/// (i.e. the executable path contains `*.app/Contents/MacOS/`).
pub fn is_bundled() -> bool {
    bundle_contents_dir().is_some()
}

/// Returns the `Contents/` directory of the enclosing `.app` bundle, or
/// `None` when running outside a bundle.
fn bundle_contents_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // Walk ancestors looking for Contents/MacOS
    let mut path = exe.as_path();
    loop {
        let parent = path.parent()?;
        if path.file_name().map(|n| n == "MacOS").unwrap_or(false)
            && parent.file_name().map(|n| n == "Contents").unwrap_or(false)
        {
            return Some(parent.to_path_buf());
        }
        path = parent;
    }
}

/// Base directory for locating assets and the binary.
///
/// - **Bundled**: `Contents/MacOS/` (executable's parent — Bevy resolves
///   its `assets/` folder relative to this directory).
/// - **Dev**: the current working directory.
pub fn base_dir() -> PathBuf {
    if let Some(contents) = bundle_contents_dir() {
        contents.join("MacOS")
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// Assets directory (`assets/`).
///
/// - **Bundled**: `Contents/MacOS/assets/` (where Bevy's AssetPlugin looks)
/// - **Dev**: `<cwd>/assets/`
pub fn assets_dir() -> PathBuf {
    base_dir().join("assets")
}

/// Configuration directory.
///
/// Resolved at startup from CLI args or OS defaults.
/// - macOS: `~/Library/Application Support/airjedi/`
/// - Linux: `~/.config/airjedi/`
/// - Windows: `%APPDATA%\airjedi\`
pub fn config_dir() -> PathBuf {
    get_paths().config.clone()
}

/// Tile and data cache directory.
///
/// Resolved at startup from CLI args or OS defaults.
/// - macOS: `~/Library/Caches/airjedi/`
/// - Linux: `~/.cache/airjedi/`
/// - Windows: `%LOCALAPPDATA%\airjedi\cache\`
pub fn cache_dir() -> PathBuf {
    get_paths().cache.clone()
}

/// Data directory for user-generated files (recordings, exports).
///
/// Resolved at startup from CLI args or OS defaults.
/// - macOS: `~/Library/Application Support/airjedi/data/`
/// - Linux: `~/.local/share/airjedi/data/`
/// - Windows: `%APPDATA%\airjedi\data\`
pub fn data_dir() -> PathBuf {
    get_paths().data.clone()
}

/// Log file directory.
///
/// Resolved at startup from CLI args or OS defaults.
/// - macOS: `~/Library/Logs/airjedi/`
/// - Linux: `~/.local/share/airjedi/logs/`
/// - Windows: `%LOCALAPPDATA%\airjedi\logs\`
pub fn log_dir() -> PathBuf {
    get_paths().log.clone()
}

/// Ensure a directory exists, creating it and all parents if necessary.
/// Returns the path unchanged for chaining.
pub fn ensure_dir(path: &Path) -> &Path {
    let _ = std::fs::create_dir_all(path);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_bundled_returns_false_in_dev() {
        assert!(!is_bundled());
    }

    #[test]
    fn test_base_dir_falls_back_to_cwd() {
        let base = base_dir();
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(base, cwd);
    }

    #[test]
    fn test_assets_dir_is_under_base_dir() {
        let assets = assets_dir();
        let base = base_dir();
        assert_eq!(assets, base.join("assets"));
    }

    #[test]
    fn test_os_defaults_returns_platform_paths() {
        let defaults = os_defaults();
        assert!(defaults.config.to_str().unwrap().contains("airjedi"));
        assert!(defaults.cache.to_str().unwrap().contains("airjedi"));
        assert!(defaults.data.to_str().unwrap().contains("airjedi"));
        assert!(defaults.log.to_str().unwrap().contains("airjedi"));
    }

    #[test]
    fn test_parse_args_defaults() {
        let paths = parse_args(vec![]).unwrap();
        let defaults = os_defaults();
        assert_eq!(paths.config, defaults.config);
        assert_eq!(paths.cache, defaults.cache);
        assert_eq!(paths.data, defaults.data);
        assert_eq!(paths.log, defaults.log);
    }

    #[test]
    fn test_parse_args_base_dir() {
        let paths = parse_args(vec![
            "--base-dir".to_string(),
            "/tmp/airjedi-test".to_string(),
        ])
        .unwrap();
        assert_eq!(paths.config, PathBuf::from("/tmp/airjedi-test/config"));
        assert_eq!(paths.cache, PathBuf::from("/tmp/airjedi-test/cache"));
        assert_eq!(paths.data, PathBuf::from("/tmp/airjedi-test/data"));
        assert_eq!(paths.log, PathBuf::from("/tmp/airjedi-test/logs"));
    }

    #[test]
    fn test_parse_args_individual_override() {
        let paths = parse_args(vec![
            "--config-dir".to_string(),
            "/tmp/my-config".to_string(),
        ])
        .unwrap();
        assert_eq!(paths.config, PathBuf::from("/tmp/my-config"));
        let defaults = os_defaults();
        assert_eq!(paths.cache, defaults.cache);
    }

    #[test]
    fn test_parse_args_individual_overrides_base_dir() {
        let paths = parse_args(vec![
            "--base-dir".to_string(),
            "/tmp/base".to_string(),
            "--config-dir".to_string(),
            "/tmp/special-config".to_string(),
        ])
        .unwrap();
        assert_eq!(paths.config, PathBuf::from("/tmp/special-config"));
        assert_eq!(paths.cache, PathBuf::from("/tmp/base/cache"));
    }

    #[test]
    fn test_parse_args_all_individual_overrides() {
        let paths = parse_args(vec![
            "--config-dir".to_string(),
            "/tmp/c".to_string(),
            "--cache-dir".to_string(),
            "/tmp/ca".to_string(),
            "--data-dir".to_string(),
            "/tmp/d".to_string(),
            "--log-dir".to_string(),
            "/tmp/l".to_string(),
        ])
        .unwrap();
        assert_eq!(paths.config, PathBuf::from("/tmp/c"));
        assert_eq!(paths.cache, PathBuf::from("/tmp/ca"));
        assert_eq!(paths.data, PathBuf::from("/tmp/d"));
        assert_eq!(paths.log, PathBuf::from("/tmp/l"));
    }

    #[test]
    fn test_parse_args_unknown_flag_errors() {
        let result = parse_args(vec!["--bogus".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown argument"));
    }

    #[test]
    fn test_parse_args_missing_value_errors() {
        let result = parse_args(vec!["--base-dir".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires a path"));
    }

    #[test]
    fn test_parse_args_missing_value_config_dir() {
        let result = parse_args(vec!["--config-dir".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_missing_value_cache_dir() {
        let result = parse_args(vec!["--cache-dir".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_missing_value_data_dir() {
        let result = parse_args(vec!["--data-dir".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_missing_value_log_dir() {
        let result = parse_args(vec!["--log-dir".to_string()]);
        assert!(result.is_err());
    }
}
