use std::fmt;

use super::pipeline::PipelineStage;

/// Context passed to providers during each fetch cycle.
/// Contains the current map state so providers can fetch data
/// relevant to the user's current view.
#[derive(Debug, Clone)]
pub struct FetchContext {
    pub center_latitude: f64,
    pub center_longitude: f64,
    pub radius_nm: f64,
}

/// Raw result from a provider's fetch operation.
pub struct RawFetchResult {
    /// Raw response bytes from the data source.
    pub data: Vec<u8>,
    /// Optional content type hint (e.g. "application/json", "text/csv").
    pub content_type: Option<String>,
    /// Source URL or identifier for logging.
    pub source: String,
}

/// Current status of a data provider.
#[derive(Debug, Clone)]
pub enum ProviderStatus {
    /// Provider has never run.
    Idle,
    /// Provider is currently fetching data.
    Fetching,
    /// Last fetch succeeded.
    Ok {
        last_success: chrono::DateTime<chrono::Utc>,
        record_count: usize,
    },
    /// Last fetch failed.
    Error {
        last_error: chrono::DateTime<chrono::Utc>,
        message: String,
    },
}

impl Default for ProviderStatus {
    fn default() -> Self {
        Self::Idle
    }
}

/// Errors that can occur during a provider fetch.
#[derive(Debug)]
pub enum ProviderError {
    /// Network or HTTP error.
    Network(String),
    /// Response parsing error.
    Parse(String),
    /// Provider-specific error.
    Other(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "network error: {}", msg),
            Self::Parse(msg) => write!(f, "parse error: {}", msg),
            Self::Other(msg) => write!(f, "provider error: {}", msg),
        }
    }
}

impl std::error::Error for ProviderError {}

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
    pub display_name: &'static str,
    pub category: ProviderCategory,
    pub description: &'static str,
    pub config_key: &'static str,
}

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

/// Trait for a data source that can fetch, parse, and transform
/// aviation data into canonical records.
pub trait DataProvider: Send + Sync {
    /// Unique name for this provider (e.g. "noaa_metar", "ourairports").
    fn name(&self) -> &str;

    /// Cron schedule expression (6-field: sec min hour dom month dow).
    /// Example: "0 */5 * * * *" = every 5 minutes.
    fn schedule(&self) -> &str;

    /// Whether this provider supports on-demand fetches outside the schedule.
    fn supports_on_demand(&self) -> bool {
        false
    }

    /// Fetch raw data from the external source.
    fn fetch(&self, ctx: &FetchContext) -> Result<RawFetchResult, ProviderError>;

    /// Return the pipeline stages this provider uses to process raw data.
    /// Stages are sorted by phase and executed in order by `run_pipeline`.
    fn pipeline_stages(&self) -> Vec<Box<dyn PipelineStage>>;

    /// Return display metadata for this provider (name, category, description).
    fn metadata(&self) -> ProviderMeta;
}

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
            assert_eq!(
                *preset, back,
                "roundtrip failed for {:?} -> {}",
                preset, cron
            );
        }
    }

    #[test]
    fn unknown_cron_maps_to_custom() {
        assert_eq!(
            SchedulePreset::from_cron("0 0 */3 * * *"),
            SchedulePreset::Custom
        );
    }
}
