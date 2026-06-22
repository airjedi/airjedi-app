# Data Ingest Settings UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an "Ingest" tab to the Tools window for managing data ingest providers with category-grouped toggles, schedule presets, live status, and wire providers into the scheduler.

**Architecture:** Extend the `DataProvider` trait with a `metadata()` method returning display info and category. Add `SchedulePreset` enum for user-friendly schedule selection. Add `ToolsTab::Ingest` to the Tools window with three collapsible category sections (Weather, Navigation, Notices). Wire `build_providers()` into `start_ingest_scheduler` to instantiate enabled providers from `DataIngestConfig`.

**Tech Stack:** Bevy 0.18, bevy_egui, egui collapsing headers, existing `DataIngestConfig`/`ProviderConfig` config system.

**Important Bevy 0.18 notes:**
- Messages use `#[derive(Message)]`, `add_message()`, `MessageWriter.write()` (NOT Event/add_event/EventWriter.send())
- Run `cargo test -p airjedi-bevy` for tests, `cargo build` for compilation checks

---

### Task 1: Add ProviderCategory and ProviderMeta to provider.rs

**Files:**
- Modify: `src/data_ingest/provider.rs`

**Step 1: Add ProviderCategory enum and ProviderMeta struct, add metadata() to DataProvider trait**

Add after the `ProviderError` impl block (after line 71) in `src/data_ingest/provider.rs`:

```rust
/// Category for grouping providers in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderCategory {
    Weather,
    Navigation,
    Notices,
}

impl ProviderCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Weather => "Weather",
            Self::Navigation => "Navigation",
            Self::Notices => "Notices",
        }
    }

    pub fn all() -> &'static [ProviderCategory] {
        &[Self::Weather, Self::Navigation, Self::Notices]
    }
}

/// Display metadata for a data provider.
pub struct ProviderMeta {
    /// Human-readable name shown in the UI.
    pub display_name: &'static str,
    /// Category for UI grouping.
    pub category: ProviderCategory,
    /// Short description of what this provider fetches.
    pub description: &'static str,
    /// Key that maps to a field in DataIngestConfig (e.g. "metar", "ourairports").
    /// Multiple providers can share the same config_key.
    pub config_key: &'static str,
}
```

Add `metadata()` method to the `DataProvider` trait (after `pipeline_stages`):

```rust
    /// Metadata for UI display and config mapping.
    fn metadata(&self) -> ProviderMeta;
```

**Step 2: Run compilation check**

Run: `cargo build 2>&1 | head -40`
Expected: Compilation errors in all provider files (missing `metadata()` impl) — that's expected, we'll fix in Task 2.

**Step 3: Commit**

```
git add src/data_ingest/provider.rs
git commit -m "Add ProviderCategory, ProviderMeta, and metadata() to DataProvider trait"
```

---

### Task 2: Implement metadata() on all 13 providers

**Files:**
- Modify: `src/data_ingest/providers/aviation_weather.rs`
- Modify: `src/data_ingest/providers/our_airports.rs`
- Modify: `src/data_ingest/providers/faa_nasr.rs`
- Modify: `src/data_ingest/providers/openaip.rs`
- Modify: `src/data_ingest/providers/notams.rs`
- Modify: `src/data_ingest/providers/tfrs.rs`

**Step 1: Add metadata() to all providers**

Add `use crate::data_ingest::provider::{..., ProviderCategory, ProviderMeta};` to each provider file's imports.

Add `metadata()` implementation to each `DataProvider` impl block:

**aviation_weather.rs** — 5 providers:

```rust
// MetarProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "METARs",
        category: ProviderCategory::Weather,
        description: "Surface weather observations",
        config_key: "metar",
    }
}

// TafProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "TAFs",
        category: ProviderCategory::Weather,
        description: "Terminal aerodrome forecasts",
        config_key: "taf",
    }
}

// SigmetProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "SIGMETs",
        category: ProviderCategory::Weather,
        description: "Significant meteorological information",
        config_key: "metar",  // shares weather config group
    }
}

// AirmetProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "AIRMETs",
        category: ProviderCategory::Weather,
        description: "Airmen meteorological information",
        config_key: "metar",  // shares weather config group
    }
}

// PirepProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "PIREPs",
        category: ProviderCategory::Weather,
        description: "Pilot weather reports",
        config_key: "metar",  // shares weather config group
    }
}
```

**our_airports.rs** — 3 providers:

```rust
// AirportsProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "Airports",
        category: ProviderCategory::Navigation,
        description: "Airport locations and details",
        config_key: "ourairports",
    }
}

// RunwaysProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "Runways",
        category: ProviderCategory::Navigation,
        description: "Runway dimensions and surfaces",
        config_key: "ourairports",
    }
}

// NavaidsProvider
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "Navaids",
        category: ProviderCategory::Navigation,
        description: "VOR, NDB, and other navigation aids",
        config_key: "ourairports",
    }
}
```

**faa_nasr.rs:**

```rust
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "FAA Airways/Freqs",
        category: ProviderCategory::Navigation,
        description: "Airways and communication frequencies from FAA NASR",
        config_key: "faa_nasr",
    }
}
```

**openaip.rs:**

```rust
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "Airspace Boundaries",
        category: ProviderCategory::Navigation,
        description: "Airspace boundaries in OpenAir format",
        config_key: "openaip",
    }
}
```

**notams.rs:**

```rust
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "NOTAMs",
        category: ProviderCategory::Notices,
        description: "Notices to Air Missions from FAA",
        config_key: "notam",
    }
}
```

**tfrs.rs:**

```rust
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "TFRs",
        category: ProviderCategory::Notices,
        description: "Temporary Flight Restrictions from FAA",
        config_key: "tfr",
    }
}
```

Also add metadata() to the `MockProvider` in `src/data_ingest/mod.rs` tests (around line 172):

```rust
fn metadata(&self) -> ProviderMeta {
    ProviderMeta {
        display_name: "Mock",
        category: ProviderCategory::Weather,
        description: "Mock provider for testing",
        config_key: "metar",
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p airjedi-bevy 2>&1 | tail -20`
Expected: All 112 tests pass.

**Step 3: Commit**

```
git add src/data_ingest/providers/ src/data_ingest/mod.rs
git commit -m "Implement metadata() on all data ingest providers"
```

---

### Task 3: Add SchedulePreset enum

**Files:**
- Modify: `src/data_ingest/provider.rs`

**Step 1: Write SchedulePreset test**

Add at the bottom of `src/data_ingest/provider.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_preset_roundtrip() {
        for preset in SchedulePreset::all() {
            if *preset == SchedulePreset::Custom {
                continue;
            }
            let cron = preset.to_cron();
            let back = SchedulePreset::from_cron(cron);
            assert_eq!(*preset, back, "roundtrip failed for {:?} -> {}", preset, cron);
        }
    }

    #[test]
    fn unknown_cron_maps_to_custom() {
        assert_eq!(SchedulePreset::from_cron("0 0 */3 * * *"), SchedulePreset::Custom);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p airjedi-bevy provider::tests 2>&1 | tail -10`
Expected: FAIL — `SchedulePreset` not defined.

**Step 3: Add SchedulePreset enum**

Add after the `ProviderMeta` struct in `src/data_ingest/provider.rs`:

```rust
/// Preset schedule intervals for the UI dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulePreset {
    Every1Min,
    Every5Min,
    Every15Min,
    Every30Min,
    Hourly,
    Daily,
    Custom,
}

impl SchedulePreset {
    pub fn to_cron(&self) -> &'static str {
        match self {
            Self::Every1Min => "0 */1 * * * *",
            Self::Every5Min => "0 */5 * * * *",
            Self::Every15Min => "0 */15 * * * *",
            Self::Every30Min => "0 */30 * * * *",
            Self::Hourly => "0 0 * * * *",
            Self::Daily => "0 0 3 * * *",
            Self::Custom => "",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Every1Min => "Every 1 min",
            Self::Every5Min => "Every 5 min",
            Self::Every15Min => "Every 15 min",
            Self::Every30Min => "Every 30 min",
            Self::Hourly => "Hourly",
            Self::Daily => "Daily",
            Self::Custom => "Custom",
        }
    }

    pub fn from_cron(cron: &str) -> Self {
        match cron {
            "0 */1 * * * *" => Self::Every1Min,
            "0 */5 * * * *" => Self::Every5Min,
            "0 */15 * * * *" => Self::Every15Min,
            "0 */30 * * * *" => Self::Every30Min,
            "0 0 * * * *" => Self::Hourly,
            "0 0 3 * * *" | "0 0 4 * * *" | "0 0 6 * * *" => Self::Daily,
            _ => Self::Custom,
        }
    }

    pub fn all() -> &'static [SchedulePreset] {
        &[
            Self::Every1Min,
            Self::Every5Min,
            Self::Every15Min,
            Self::Every30Min,
            Self::Hourly,
            Self::Daily,
            Self::Custom,
        ]
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p airjedi-bevy provider::tests 2>&1 | tail -10`
Expected: 2 tests pass.

**Step 5: Commit**

```
git add src/data_ingest/provider.rs
git commit -m "Add SchedulePreset enum with cron roundtrip conversion"
```

---

### Task 4: Enhance IngestStatus resource

**Files:**
- Modify: `src/data_ingest/mod.rs`

**Step 1: Update ProviderStatusEntry to carry metadata**

In `src/data_ingest/mod.rs`, update the `ProviderStatusEntry` struct (around line 42):

```rust
/// Status entry for a single provider.
pub struct ProviderStatusEntry {
    pub name: String,
    pub display_name: String,
    pub category: provider::ProviderCategory,
    pub description: String,
    pub config_key: String,
    pub status: provider::ProviderStatus,
}
```

**Step 2: Add a helper to build initial status from providers**

Add an `impl IngestStatus` block:

```rust
impl IngestStatus {
    /// Build initial status entries from a list of providers.
    pub fn from_providers(providers: &[std::sync::Arc<dyn provider::DataProvider>]) -> Self {
        let entries = providers
            .iter()
            .map(|p| {
                let meta = p.metadata();
                ProviderStatusEntry {
                    name: p.name().to_string(),
                    display_name: meta.display_name.to_string(),
                    category: meta.category,
                    description: meta.description.to_string(),
                    config_key: meta.config_key.to_string(),
                    status: provider::ProviderStatus::Idle,
                }
            })
            .collect();
        Self { providers: entries }
    }
}
```

**Step 3: Run build**

Run: `cargo build 2>&1 | tail -20`
Expected: Compiles. (The scheduler doesn't populate status yet — that's fine.)

**Step 4: Commit**

```
git add src/data_ingest/mod.rs
git commit -m "Enhance IngestStatus with provider metadata for UI display"
```

---

### Task 5: Add build_providers() and wire into scheduler startup

**Files:**
- Modify: `src/data_ingest/mod.rs`

**Step 1: Add build_providers function**

Add before `start_ingest_scheduler` in `src/data_ingest/mod.rs`:

```rust
/// Instantiate data providers based on config flags.
fn build_providers(config: &crate::config::DataIngestConfig) -> Vec<Arc<dyn provider::DataProvider>> {
    let mut providers: Vec<Arc<dyn provider::DataProvider>> = vec![];

    if config.metar.enabled {
        providers.push(Arc::new(providers::aviation_weather::MetarProvider));
        providers.push(Arc::new(providers::aviation_weather::TafProvider));
        providers.push(Arc::new(providers::aviation_weather::SigmetProvider));
        providers.push(Arc::new(providers::aviation_weather::AirmetProvider));
        providers.push(Arc::new(providers::aviation_weather::PirepProvider));
    } else {
        // Allow individual taf override
        if config.taf.enabled {
            providers.push(Arc::new(providers::aviation_weather::TafProvider));
        }
    }

    if config.ourairports.enabled {
        providers.push(Arc::new(providers::our_airports::AirportsProvider));
        providers.push(Arc::new(providers::our_airports::RunwaysProvider));
        providers.push(Arc::new(providers::our_airports::NavaidsProvider));
    }

    if config.faa_nasr.enabled {
        providers.push(Arc::new(providers::faa_nasr::FaaNasrProvider::new()));
    }

    if config.openaip.enabled {
        providers.push(Arc::new(providers::openaip::OpenAipProvider::new()));
    }

    if config.notam.enabled {
        providers.push(Arc::new(providers::notams::NotamProvider));
    }

    if config.tfr.enabled {
        providers.push(Arc::new(providers::tfrs::TfrProvider));
    }

    providers
}
```

**Step 2: Update start_ingest_scheduler to use build_providers**

Replace the line `let providers: Vec<Arc<dyn provider::DataProvider>> = vec![];` (line 104) with:

```rust
    let providers = build_providers(&app_config.data_ingest);
```

And insert the IngestStatus resource after building providers:

```rust
    let ingest_status = IngestStatus::from_providers(&providers);
    // ... after commands.insert_resource(IngestSender { tx }):
    commands.insert_resource(ingest_status);
```

Remove the existing `app.init_resource::<IngestStatus>()` from the plugin `build()` method since we now insert it in the startup system.

**Step 3: Run build**

Run: `cargo build 2>&1 | tail -20`
Expected: Compiles.

**Step 4: Run tests**

Run: `cargo test -p airjedi-bevy 2>&1 | tail -20`
Expected: All tests pass.

**Step 5: Commit**

```
git add src/data_ingest/mod.rs
git commit -m "Wire providers into scheduler from DataIngestConfig"
```

---

### Task 6: Add Ingest tab to Tools window

**Files:**
- Modify: `src/tools_window.rs`
- Modify: `src/data_ingest/mod.rs` (re-export needed types)

**Step 1: Add ToolsTab::Ingest variant**

In `src/tools_window.rs`, add `Ingest` to the `ToolsTab` enum (after `View3D`):

```rust
    Ingest,
```

**Step 2: Add the tab button to the tab bar**

In `render_tools_window`, in the horizontal tab bar (around line 99), add after the "3D View" tab:

```rust
                if ui.selectable_label(tools_state.active_tab == ToolsTab::Ingest, "Ingest").clicked() {
                    tools_state.active_tab = ToolsTab::Ingest;
                }
```

**Step 3: Add render dispatch and system parameter**

Add `IngestStatus` and `AppConfig` as parameters to `render_tools_window`:

```rust
    app_config: Res<crate::config::AppConfig>,
    ingest_status: Option<Res<crate::data_ingest::IngestStatus>>,
```

Add the match arm inside the `ScrollArea` (after `ToolsTab::View3D`):

```rust
                        ToolsTab::Ingest => render_ingest_tab(ui, ingest_status.as_deref(), &app_config),
```

**Step 4: Write the render_ingest_tab function**

Add at the bottom of `src/tools_window.rs` (before `#[cfg(test)]`):

```rust
pub fn render_ingest_tab(
    ui: &mut egui::Ui,
    ingest_status: Option<&crate::data_ingest::IngestStatus>,
    app_config: &crate::config::AppConfig,
) {
    use crate::data_ingest::provider::{ProviderCategory, ProviderStatus, SchedulePreset};

    let Some(status) = ingest_status else {
        ui.label("Data ingest not initialized");
        return;
    };

    if status.providers.is_empty() {
        ui.label("No providers enabled. Enable providers below and restart.");
        ui.separator();
    }

    // Group providers by category, show config for all known providers
    for category in ProviderCategory::all() {
        let category_providers: Vec<_> = status
            .providers
            .iter()
            .filter(|p| p.category == *category)
            .collect();

        // Always show category even if empty (providers might be disabled)
        egui::CollapsingHeader::new(category.display_name())
            .default_open(true)
            .show(ui, |ui| {
                if category_providers.is_empty() {
                    ui.label(
                        egui::RichText::new("No providers active in this category")
                            .size(11.0)
                            .color(egui::Color32::GRAY),
                    );
                    return;
                }

                for provider in &category_providers {
                    ui.horizontal(|ui| {
                        // Status dot
                        let (color, status_text) = match &provider.status {
                            ProviderStatus::Idle => (egui::Color32::GRAY, "Idle".to_string()),
                            ProviderStatus::Fetching => (egui::Color32::YELLOW, "Fetching...".to_string()),
                            ProviderStatus::Ok { last_success, record_count } => {
                                let time = last_success.format("%H:%M").to_string();
                                (egui::Color32::GREEN, format!("{} ({})", time, record_count))
                            }
                            ProviderStatus::Error { message, .. } => {
                                (egui::Color32::RED, format!("Err: {}", message))
                            }
                        };

                        ui.colored_label(color, "\u{25CF}");
                        ui.label(&provider.display_name);
                        ui.label(
                            egui::RichText::new(&status_text)
                                .size(10.0)
                                .color(egui::Color32::GRAY),
                        );
                    });

                    // Schedule info
                    let config_schedule = get_provider_schedule(app_config, &provider.config_key);
                    let preset = SchedulePreset::from_cron(&config_schedule);
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new(format!("Schedule: {}", preset.display_name()))
                                .size(10.0)
                                .color(egui::Color32::from_rgb(150, 150, 150)),
                        );
                    });
                }
            });

        ui.add_space(4.0);
    }

    // Config section
    ui.separator();
    ui.label(
        egui::RichText::new("Enable/disable providers in Settings > config.toml")
            .size(10.0)
            .color(egui::Color32::GRAY),
    );
}

/// Look up the schedule string for a provider config key.
fn get_provider_schedule(config: &crate::config::AppConfig, config_key: &str) -> String {
    match config_key {
        "metar" => config.data_ingest.metar.schedule.clone(),
        "taf" => config.data_ingest.taf.schedule.clone(),
        "ourairports" => config.data_ingest.ourairports.schedule.clone(),
        "faa_nasr" => config.data_ingest.faa_nasr.schedule.clone(),
        "openaip" => config.data_ingest.openaip.schedule.clone(),
        "notam" => config.data_ingest.notam.schedule.clone(),
        "tfr" => config.data_ingest.tfr.schedule.clone(),
        _ => "unknown".to_string(),
    }
}
```

**Step 5: Run build**

Run: `cargo build 2>&1 | tail -20`
Expected: Compiles.

**Step 6: Commit**

```
git add src/tools_window.rs
git commit -m "Add Ingest tab to Tools window with category-grouped provider status"
```

---

### Task 7: Fix validate_and_build to preserve data_ingest config

**Files:**
- Modify: `src/config.rs`

**Step 1: Fix validate_and_build()**

In `src/config.rs`, the `validate_and_build()` method (around line 446) currently returns `data_ingest: DataIngestConfig::default()`. This means saving settings resets all ingest config to defaults.

Add a `data_ingest` field to `SettingsUiState`:

```rust
pub struct SettingsUiState {
    // ... existing fields ...
    pub data_ingest: DataIngestConfig,
}
```

In `populate_from_config`:

```rust
    self.data_ingest = config.data_ingest.clone();
```

In `validate_and_build`, change the `data_ingest` field in the returned `AppConfig`:

```rust
    data_ingest: self.data_ingest.clone(),
```

**Step 2: Run build**

Run: `cargo build 2>&1 | tail -20`
Expected: Compiles.

**Step 3: Run tests**

Run: `cargo test -p airjedi-bevy 2>&1 | tail -20`
Expected: All tests pass.

**Step 4: Commit**

```
git add src/config.rs
git commit -m "Preserve data_ingest config when saving settings"
```

---

### Task 8: Run app and verify Ingest tab

**Step 1: Run the app**

Run: `cargo run --release 2>&1 | head -30`

**Step 2: Verify**

- Open Tools window (keyboard shortcut or toolbar)
- Click "Ingest" tab
- Verify three category headers appear (Weather, Navigation, Notices)
- Verify providers show based on default config (METAR, TAF, OurAirports, TFR enabled by default)
- Verify status dots show gray (Idle) since providers haven't fetched yet
- Verify schedule presets display correctly

**Step 3: Commit (if any fixes needed)**

Only commit if fixes were required during verification.
