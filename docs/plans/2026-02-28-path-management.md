# Path Management Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make tile caching and app configuration use consistent OS-standard paths with CLI override support.

**Architecture:** A `static OnceLock<AppPaths>` initialized from CLI args at the top of `main()` before Bevy starts. All path functions in `paths.rs` read from this lock. `tile_cache.rs` stops calling `dirs::cache_dir()` directly and goes through `paths.rs`. CLI parsing is manual `std::env::args` with `--base-dir` and per-directory flags.

**Tech Stack:** `std::sync::OnceLock`, `std::env::args`, `dirs` crate (already a dependency)

**Design doc:** `docs/plans/2026-02-28-path-management-design.md`

---

### Task 1: Add `AppPaths` struct and `OnceLock` to `paths.rs`

**Files:**
- Modify: `src/paths.rs`

**Step 1: Add the `AppPaths` struct and static lock**

Add at the top of `src/paths.rs`, after the existing imports:

```rust
use std::sync::OnceLock;

/// Resolved application directory paths, initialized once at startup.
pub struct AppPaths {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
    pub log: PathBuf,
}

static PATHS: OnceLock<AppPaths> = OnceLock::new();

/// Initialize application paths. Must be called once at startup before
/// any path functions are used. Panics if called more than once.
pub fn init(paths: AppPaths) {
    PATHS.set(paths).expect("paths::init called more than once");
}

fn get_paths() -> &'static AppPaths {
    PATHS.get().expect("paths::init was not called before accessing paths")
}
```

**Step 2: Add OS-default path builder**

Add a function that builds default OS paths (no CLI overrides):

```rust
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
```

**Step 3: Update existing path functions to use the lock**

Replace the bodies of `config_dir()`, `data_dir()`, and `log_dir()`:

```rust
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
```

Keep `is_bundled()`, `bundle_contents_dir()`, `base_dir()`, `assets_dir()`, and `ensure_dir()` unchanged -- `assets_dir()` still needs bundle detection for Bevy's AssetPlugin.

**Step 4: Build and verify**

Run: `cargo build 2>&1 | head -30`
Expected: Build errors from tests that reference old behavior (we'll fix those later). Core code should compile.

**Step 5: Commit**

```
feat: add AppPaths struct with OnceLock for consistent path resolution
```

---

### Task 2: Add CLI argument parsing

**Files:**
- Modify: `src/paths.rs`

**Step 1: Add the CLI parser and help text**

Add to `src/paths.rs`:

```rust
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
        Ok(p) => init(p),
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
    let mut config_dir: Option<PathBuf> = None;
    let mut cache_dir: Option<PathBuf> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut log_dir: Option<PathBuf> = None;

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
                config_dir = Some(PathBuf::from(
                    iter.next().ok_or("--config-dir requires a path argument")?,
                ));
            }
            "--cache-dir" => {
                cache_dir = Some(PathBuf::from(
                    iter.next().ok_or("--cache-dir requires a path argument")?,
                ));
            }
            "--data-dir" => {
                data_dir = Some(PathBuf::from(
                    iter.next().ok_or("--data-dir requires a path argument")?,
                ));
            }
            "--log-dir" => {
                log_dir = Some(PathBuf::from(
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
        config: config_dir
            .or_else(|| base_dir.as_ref().map(|b| b.join("config")))
            .unwrap_or(defaults.config),
        cache: cache_dir
            .or_else(|| base_dir.as_ref().map(|b| b.join("cache")))
            .unwrap_or(defaults.cache),
        data: data_dir
            .or_else(|| base_dir.as_ref().map(|b| b.join("data")))
            .unwrap_or(defaults.data),
        log: log_dir
            .or_else(|| base_dir.as_ref().map(|b| b.join("logs")))
            .unwrap_or(defaults.log),
    })
}
```

**Step 2: Build and verify**

Run: `cargo build 2>&1 | head -30`
Expected: Compiles. Tests may still fail (OnceLock not initialized in test context).

**Step 3: Commit**

```
feat: add CLI argument parsing for path overrides
```

---

### Task 3: Wire up `main.rs` to call `init_from_args()`

**Files:**
- Modify: `src/main.rs`

**Step 1: Add `init_from_args()` call at the top of `main()`**

In `src/main.rs`, add as the very first line of `fn main()`, before the `#[cfg(unix)]` file descriptor limit block:

```rust
paths::init_from_args();
```

**Step 2: Build and run**

Run: `cargo build`
Expected: Compiles clean.

Run: `cargo run -- --help`
Expected: Prints help text and exits.

**Step 3: Commit**

```
feat: initialize paths from CLI arguments at startup
```

---

### Task 4: Update `tile_cache.rs` to use `paths::cache_dir()`

**Files:**
- Modify: `src/tile_cache.rs`

**Step 1: Replace `tile_cache_dir()` implementation**

Change the `tile_cache_dir()` function from calling `dirs::cache_dir()` directly to using `paths::cache_dir()`:

```rust
/// Returns the tile cache directory.
///
/// Uses the app-wide cache directory with a `tiles` subdirectory.
/// The cache directory is resolved at startup from CLI args or OS defaults.
pub fn tile_cache_dir() -> PathBuf {
    crate::paths::cache_dir().join("tiles")
}
```

Remove the `use std::path::{Path, PathBuf};` line since `Path` is still used but `PathBuf` for the dirs call is no longer needed here -- actually check if `Path` and `PathBuf` are used elsewhere in the file. They are (`Path` in function signatures, `PathBuf` in return types), so keep the import.

**Step 2: Build and verify**

Run: `cargo build`
Expected: Compiles clean. Tile cache behavior unchanged at runtime (same resolved path).

**Step 3: Commit**

```
refactor: route tile cache dir through paths module
```

---

### Task 5: Update tests in `paths.rs`

**Files:**
- Modify: `src/paths.rs`

**Step 1: Update the test module**

The existing tests assume dev-mode behavior (config_dir == cwd). Since we now always use OS paths and require `init()` to be called, the tests need updating. Since `OnceLock` can only be set once per process and tests run in the same process, use `parse_args` directly to test argument parsing:

```rust
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
        // All paths should contain "airjedi"
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
        ]).unwrap();
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
        ]).unwrap();
        assert_eq!(paths.config, PathBuf::from("/tmp/my-config"));
        // Others should be OS defaults
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
        ]).unwrap();
        // Individual flag wins over base-dir
        assert_eq!(paths.config, PathBuf::from("/tmp/special-config"));
        // Others use base-dir
        assert_eq!(paths.cache, PathBuf::from("/tmp/base/cache"));
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
}
```

**Step 2: Run tests**

Run: `cargo test -p airjedi-bevy -- paths::tests`
Expected: All tests pass.

**Step 3: Commit**

```
test: update path tests for OnceLock and CLI arg parsing
```

---

### Task 6: Log resolved paths at startup

**Files:**
- Modify: `src/paths.rs`

**Step 1: Add startup path logging to `init_from_args()`**

After the `init(p)` call in `init_from_args()`, add logging so users can see where files live:

```rust
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
```

Use `eprintln!` since Bevy's logger isn't initialized yet at this point.

**Step 2: Build and test**

Run: `cargo run 2>&1 | head -10`
Expected: See the resolved paths printed before Bevy startup logs.

Run: `cargo run -- --base-dir /tmp/airjedi-test 2>&1 | head -10`
Expected: See paths under `/tmp/airjedi-test/`.

**Step 3: Commit**

```
feat: log resolved paths at startup
```

---

### Task 7: Final build and manual verification

**Files:** None (verification only)

**Step 1: Full build**

Run: `cargo build`
Expected: Clean compile, no warnings related to paths.

**Step 2: Run all tests**

Run: `cargo test`
Expected: All tests pass.

**Step 3: Verify default behavior**

Run: `cargo run`
Expected: App starts normally. Config is read from/written to `~/Library/Application Support/airjedi/config.toml`. Tile cache uses `~/Library/Caches/airjedi/tiles/`. Startup logs show resolved paths.

**Step 4: Verify CLI overrides**

Run: `cargo run -- --base-dir /tmp/airjedi-test`
Expected: App starts with all paths under `/tmp/airjedi-test/`. A new `config.toml` is created at `/tmp/airjedi-test/config/config.toml` with defaults.

Run: `cargo run -- --cache-dir /tmp/test-cache`
Expected: Tiles cached to `/tmp/test-cache/tiles/` while config stays at OS default.

Run: `cargo run -- --help`
Expected: Prints help and exits.

Run: `cargo run -- --bogus`
Expected: Prints error and help, exits with code 1.

**Step 5: Commit if any fixes were needed**

```
fix: address issues found during verification
```
