# Path Management: Consistent OS Paths with CLI Overrides

## Problem

The tile cache and app configuration use inconsistent path strategies:

- **Tile cache** always uses OS-standard paths (`~/Library/Caches/airjedi/tiles`) via `dirs::cache_dir()` directly in `tile_cache.rs`, bypassing `paths.rs`.
- **App config** in dev mode writes to `<cwd>/config.toml`, which breaks when launched from a different directory. In bundled mode it uses `~/Library/Application Support/airjedi/`.
- **Data and log dirs** in dev mode use `<cwd>/tmp/`, also CWD-dependent.
- No CLI argument support for overriding any of these paths.

## Design

### Always use OS-standard paths

Remove the `is_bundled()` branching from `paths.rs`. All modes (dev, bundled, release) use the same OS-standard directories:

| Directory | macOS | Linux | Windows |
|-----------|-------|-------|---------|
| Config | `~/Library/Application Support/airjedi/` | `~/.config/airjedi/` | `%APPDATA%\airjedi\` |
| Cache | `~/Library/Caches/airjedi/` | `~/.cache/airjedi/` | `%LOCALAPPDATA%\airjedi\cache\` |
| Data | `~/Library/Application Support/airjedi/data/` | `~/.local/share/airjedi/data/` | `%APPDATA%\airjedi\data\` |
| Logs | `~/Library/Logs/airjedi/` | `~/.local/share/airjedi/logs/` | `%LOCALAPPDATA%\airjedi\logs\` |

### CLI flags (manual `std::env::args`)

- `--base-dir <path>` -- root for all paths; subdirectories are `config/`, `cache/`, `data/`, `logs/`
- `--config-dir <path>` -- override config directory
- `--cache-dir <path>` -- override cache directory
- `--data-dir <path>` -- override data directory
- `--log-dir <path>` -- override log directory
- `--help` / `-h` -- print usage and exit

Priority: explicit flag > `--base-dir` subdirectory > OS default.

### Architecture: `OnceLock<AppPaths>`

Paths are resolved once at startup (before `App::new()`) and stored in a `static OnceLock<AppPaths>`. All path functions read from this lock.

```rust
pub struct AppPaths {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
    pub log: PathBuf,
}

static PATHS: OnceLock<AppPaths> = OnceLock::new();

pub fn init(paths: AppPaths) { PATHS.set(paths).ok(); }
pub fn config_dir() -> PathBuf { PATHS.get().unwrap().config.clone() }
pub fn cache_dir() -> PathBuf { PATHS.get().unwrap().cache.clone() }
pub fn data_dir() -> PathBuf { PATHS.get().unwrap().data.clone() }
pub fn log_dir() -> PathBuf { PATHS.get().unwrap().log.clone() }
```

### File changes

1. **`src/paths.rs`**: Replace `is_bundled()`-based functions with `OnceLock<AppPaths>`. Add `init()`, `parse_cli_args()`, and `cache_dir()`. Keep `assets_dir()`, `base_dir()`, `ensure_dir()` unchanged (assets still need bundle detection for Bevy's AssetPlugin).

2. **`src/tile_cache.rs`**: Change `tile_cache_dir()` to call `crate::paths::cache_dir().join("tiles")` instead of `dirs::cache_dir()` directly.

3. **`src/main.rs`**: Call `paths::init_from_args()` at the top of `main()` before `App::new()`.

### Migration

On first run after this change, users with existing `./config.toml` in the project root will need to move it to the OS config directory. The app will log the expected config path on startup to help with this. No automatic migration -- the old file is just ignored and defaults are created in the new location.

### `--help` output

```
AirJedi - Aircraft Map Tracker

USAGE: airjedi [OPTIONS]

OPTIONS:
    --base-dir <PATH>    Base directory for all app data (config/cache/data/logs)
    --config-dir <PATH>  Configuration directory (default: OS standard)
    --cache-dir <PATH>   Tile cache directory (default: OS standard)
    --data-dir <PATH>    Data directory for recordings/exports (default: OS standard)
    --log-dir <PATH>     Log file directory (default: OS standard)
    -h, --help           Print this help message
```
