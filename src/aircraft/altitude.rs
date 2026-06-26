use crate::geo::FL_THRESHOLD;

/// Format altitude for display. At or above FL_THRESHOLD (18,000 ft),
/// displays as flight level (e.g. "FL350"); below, as feet (e.g. "12500 ft").
/// Returns "---" for None.
pub fn format_altitude(alt: Option<i32>) -> String {
    match alt {
        Some(a) if a >= FL_THRESHOLD => format!("FL{:03}", a / 100),
        Some(a) => format!("{} ft", a),
        None => "---".to_string(),
    }
}

/// Format altitude with a vertical-rate indicator prefix.
/// Suitable for compact list displays.
pub fn format_altitude_with_indicator(alt: i32, indicator: &str) -> String {
    if alt >= FL_THRESHOLD {
        format!("{} FL{:03}", indicator, alt / 100)
    } else {
        format!("{} {} ft", indicator, alt)
    }
}
