# Data Ingest Settings UI Design

## Summary

Add an "Ingest" tab to the Tools window for managing data ingest providers. Providers are grouped by category (Weather, Navigation, Notices) with per-source enable/disable toggles, schedule presets, and live status display. Also wires providers into the scheduler (currently empty).

## Provider Metadata

Add `metadata()` to the `DataProvider` trait returning `ProviderMeta`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCategory {
    Weather,
    Navigation,
    Notices,
}

pub struct ProviderMeta {
    pub display_name: &'static str,
    pub category: ProviderCategory,
    pub description: &'static str,
    pub config_key: &'static str, // maps to DataIngestConfig field name
}
```

Multiple providers can share a `config_key` (e.g., all OurAirports sub-providers share `"ourairports"`), so one config toggle controls the group.

## Config Structure

Keep the existing `DataIngestConfig` with named fields (metar, taf, ourairports, faa_nasr, openaip, notam, tfr). Each is a `ProviderConfig { enabled: bool, schedule: String }`. No structural changes needed.

Fix `validate_and_build()` in `SettingsUiState` to preserve `data_ingest` config instead of defaulting it.

## Schedule Presets

```rust
pub enum SchedulePreset {
    Every1Min, Every5Min, Every15Min, Every30Min, Hourly, Daily, Custom,
}
```

Each preset maps to/from a 6-field cron expression. The UI defaults to preset dropdowns. An "Advanced" checkbox reveals the raw cron text field.

## IngestStatus Resource

Enhance the existing `IngestStatus` resource to carry display metadata:

```rust
pub struct ProviderStatusEntry {
    pub name: String,
    pub display_name: String,
    pub category: ProviderCategory,
    pub description: String,
    pub config_key: String,
    pub status: ProviderStatus,
}
```

Populated at startup from provider metadata. Status updates flow from the scheduler thread.

## UI Layout

New `ToolsTab::Ingest` in the Tools window:

```
┌─ Tools ──────────────────────────────────────────────┐
│ Coverage | Airspace | Sources | Export | Rec | 3D | Ingest │
├──────────────────────────────────────────────────────┤
│ ▼ Weather                                            │
│   ☑ METARs          every 5 min    ● OK  12:05 (42)  │
│   ☑ TAFs            every 15 min   ● OK  12:00 (8)   │
│   ☐ SIGMETs         every 15 min   ○ Idle            │
│   ☐ AIRMETs         every 15 min   ○ Idle            │
│   ☐ PIREPs          every 5 min    ○ Idle            │
│                                                      │
│ ▼ Navigation                                         │
│   ☑ OurAirports     daily          ● OK  03:00 (1.2k)│
│   ☐ FAA NASR        daily          ○ Idle            │
│   ☐ OpenAIP Airspace daily         ○ Idle            │
│                                                      │
│ ▼ Notices                                            │
│   ☐ NOTAMs          every 30 min   ○ Idle            │
│   ☑ TFRs            every 15 min   ● OK  12:10 (3)   │
│                                                      │
│ ☐ Show advanced (cron expressions)                   │
│                                     [Save] [Fetch All]│
└──────────────────────────────────────────────────────┘
```

Status dots: green=OK, yellow=fetching, red=error, gray=idle/disabled.

## Provider Wiring

Add `build_providers()` that reads `DataIngestConfig` and instantiates enabled providers:

```rust
fn build_providers(config: &DataIngestConfig) -> Vec<Arc<dyn DataProvider>> {
    let mut providers: Vec<Arc<dyn DataProvider>> = vec![];
    if config.metar.enabled { providers.push(Arc::new(MetarProvider)); }
    if config.taf.enabled { providers.push(Arc::new(TafProvider)); }
    // ... etc for all providers
    providers
}
```

Called by `start_ingest_scheduler` at startup.

## Data Flow

1. App starts -> `start_ingest_scheduler` reads `DataIngestConfig`, calls `build_providers()`, spawns scheduler thread
2. Scheduler populates `IngestStatus` with provider metadata
3. Scheduler runs providers on cron schedule, updates `ProviderStatus` via crossbeam channel
4. Ingest tab reads `IngestStatus` + `DataIngestConfig` to render UI
5. User changes settings -> updates `DataIngestConfig` -> saves to TOML -> signals scheduler reload

## Files Changed

- `src/data_ingest/provider.rs` — Add `ProviderCategory`, `ProviderMeta`, `metadata()` to trait, `SchedulePreset`
- `src/data_ingest/providers/*.rs` — Implement `metadata()` on all 13 providers
- `src/data_ingest/mod.rs` — Enhance `IngestStatus`/`ProviderStatusEntry`, add `build_providers()`
- `src/tools_window.rs` — Add `ToolsTab::Ingest`, `render_ingest_tab()`
- `src/config.rs` — Fix `validate_and_build()` to preserve data_ingest config
