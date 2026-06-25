use bevy::prelude::*;
use serde::Deserialize;

/// Airport type from OurAirports
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirportType {
    LargeAirport,
    MediumAirport,
    SmallAirport,
    Heliport,
    SeaplaneBase,
    Balloonport,
    Closed,
    #[serde(other)]
    Unknown,
}

/// Airport filter mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AirportFilter {
    /// All public airports
    All,
    /// Airports with scheduled service (default)
    #[default]
    FrequentlyUsed,
    /// Large international airports only
    MajorOnly,
}

/// Airport data from OurAirports
#[derive(Debug, Clone, Deserialize)]
pub struct Airport {
    pub id: i64,
    pub ident: String,
    #[serde(rename = "type")]
    pub airport_type: AirportType,
    pub name: String,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub elevation_ft: Option<i32>,
    pub continent: Option<String>,
    pub iso_country: Option<String>,
    pub iso_region: Option<String>,
    pub municipality: Option<String>,
    pub scheduled_service: Option<String>,
    pub gps_code: Option<String>,
    pub iata_code: Option<String>,
    pub local_code: Option<String>,
}

impl Airport {
    /// Check if airport has scheduled service
    pub fn has_scheduled_service(&self) -> bool {
        self.scheduled_service.as_deref() == Some("yes")
    }

    /// Check if airport is a major (large) airport
    pub fn is_major(&self) -> bool {
        self.airport_type == AirportType::LargeAirport
    }

    /// Check if airport passes the current filter
    pub fn passes_filter(&self, filter: AirportFilter) -> bool {
        match filter {
            AirportFilter::All => matches!(
                self.airport_type,
                AirportType::LargeAirport | AirportType::MediumAirport | AirportType::SmallAirport
            ),
            AirportFilter::FrequentlyUsed => self.has_scheduled_service(),
            AirportFilter::MajorOnly => self.is_major(),
        }
    }

    /// Get color based on airport size
    pub fn color(&self) -> Color {
        match self.airport_type {
            AirportType::LargeAirport => Color::srgb(1.0, 0.2, 0.2), // Red
            AirportType::MediumAirport => Color::srgb(1.0, 0.6, 0.2), // Orange
            _ => Color::srgb(0.6, 0.6, 0.6),                         // Gray
        }
    }
}

/// Runway data from OurAirports
#[derive(Debug, Clone, Deserialize)]
pub struct Runway {
    pub id: i64,
    pub airport_ref: i64,
    pub airport_ident: String,
    pub length_ft: Option<i32>,
    pub width_ft: Option<i32>,
    pub surface: Option<String>,
    pub lighted: Option<i32>,
    pub closed: Option<i32>,
    pub le_ident: Option<String>,
    pub le_latitude_deg: Option<f64>,
    pub le_longitude_deg: Option<f64>,
    pub le_elevation_ft: Option<i32>,
    #[serde(rename = "le_heading_degT")]
    pub le_heading_deg_t: Option<f64>,
    pub he_ident: Option<String>,
    pub he_latitude_deg: Option<f64>,
    pub he_longitude_deg: Option<f64>,
    pub he_elevation_ft: Option<i32>,
    #[serde(rename = "he_heading_degT")]
    pub he_heading_deg_t: Option<f64>,
}

impl Runway {
    /// Check if runway has valid coordinates for both ends
    pub fn has_valid_coords(&self) -> bool {
        self.le_latitude_deg.is_some()
            && self.le_longitude_deg.is_some()
            && self.he_latitude_deg.is_some()
            && self.he_longitude_deg.is_some()
    }

    /// Check if runway is closed
    pub fn is_closed(&self) -> bool {
        self.closed == Some(1)
    }
}

/// Navaid type
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum NavaidType {
    Vor,
    #[serde(alias = "VOR-DME")]
    VorDme,
    Dme,
    Ndb,
    #[serde(alias = "NDB-DME")]
    NdbDme,
    Tacan,
    #[serde(alias = "VORTAC")]
    Vortac,
    #[serde(other)]
    Unknown,
}

/// Navaid data from OurAirports
#[derive(Debug, Clone, Deserialize)]
pub struct Navaid {
    pub id: i64,
    pub filename: Option<String>,
    pub ident: String,
    pub name: String,
    #[serde(rename = "type")]
    pub navaid_type: NavaidType,
    pub frequency_khz: Option<i32>,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub elevation_ft: Option<i32>,
    pub iso_country: Option<String>,
    pub dme_frequency_khz: Option<i32>,
    pub dme_channel: Option<String>,
    pub dme_latitude_deg: Option<f64>,
    pub dme_longitude_deg: Option<f64>,
    pub dme_elevation_ft: Option<i32>,
    pub slaved_variation_deg: Option<f64>,
    pub magnetic_variation_deg: Option<f64>,
    #[serde(rename = "usageType")]
    pub usage_type: Option<String>,
    pub power: Option<String>,
    pub associated_airport: Option<String>,
}

impl Navaid {
    /// Get color based on navaid type
    pub fn color(&self) -> Color {
        match self.navaid_type {
            NavaidType::Vor | NavaidType::VorDme | NavaidType::Vortac => {
                Color::srgb(0.2, 0.6, 1.0) // Blue
            }
            NavaidType::Ndb | NavaidType::NdbDme => {
                Color::srgb(1.0, 0.6, 0.2) // Orange
            }
            NavaidType::Dme | NavaidType::Tacan => {
                Color::srgb(0.8, 0.2, 1.0) // Purple
            }
            NavaidType::Unknown => Color::srgb(0.5, 0.5, 0.5), // Gray
        }
    }

    /// Get frequency as string for display
    pub fn frequency_display(&self) -> String {
        if let Some(freq) = self.frequency_khz {
            if freq >= 1000 {
                format!("{:.2}", freq as f64 / 1000.0)
            } else {
                freq.to_string()
            }
        } else {
            String::new()
        }
    }
}
