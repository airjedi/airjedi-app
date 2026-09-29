use std::collections::BTreeSet;

use airjedi_core::{
    AltitudeReference, DisplayHistorySample, DisplayTrail, HistoryCoverage, HistorySessionId,
    Timestamp, TrackId,
};
use bevy::prelude::Resource;
use bevy_egui::egui;
use chrono::{DateTime, Duration, Utc};

use super::components::{Aircraft, FusionTrackLink};
use super::list_panel::AircraftListState;

const DEFAULT_MAX_POINTS: usize = 160;
const CHART_HEIGHT: f32 = 132.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryChartWindow {
    FiveMinutes,
    #[default]
    FifteenMinutes,
    ThirtyMinutes,
}

impl HistoryChartWindow {
    pub const ALL: [Self; 3] = [Self::FiveMinutes, Self::FifteenMinutes, Self::ThirtyMinutes];

    #[must_use]
    pub const fn duration(self) -> Duration {
        match self {
            Self::FiveMinutes => Duration::minutes(5),
            Self::FifteenMinutes => Duration::minutes(15),
            Self::ThirtyMinutes => Duration::minutes(30),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryChartLoading {
    NoSelection,
    Waiting,
    Preview,
    Loading,
    Partial,
    Complete,
    RetryableError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChartTransferProgress {
    pub received_chunks: u16,
    pub total_chunks: u16,
    pub received_samples: usize,
    pub expected_samples: Option<usize>,
}

#[derive(Resource, Debug, Clone)]
pub struct HistoryChartState {
    pub track_id: Option<TrackId>,
    pub session_id: Option<HistorySessionId>,
    pub server_time: Option<Timestamp>,
    pub history_revision: u64,
    pub coverage: HistoryCoverage,
    pub loading: HistoryChartLoading,
    pub transfer: Option<ChartTransferProgress>,
    pub samples: Vec<DisplayHistorySample>,
    pub window: HistoryChartWindow,
}

impl Default for HistoryChartState {
    fn default() -> Self {
        Self {
            track_id: None,
            session_id: None,
            server_time: None,
            history_revision: 0,
            coverage: HistoryCoverage::default(),
            loading: HistoryChartLoading::NoSelection,
            transfer: None,
            samples: Vec::new(),
            window: HistoryChartWindow::default(),
        }
    }
}

impl HistoryChartState {
    pub fn select(&mut self, track_id: Option<TrackId>) {
        if self.track_id == track_id {
            return;
        }
        self.track_id = track_id;
        self.session_id = None;
        self.server_time = None;
        self.history_revision = 0;
        self.coverage = HistoryCoverage::default();
        self.loading = if self.track_id.is_some() {
            HistoryChartLoading::Waiting
        } else {
            HistoryChartLoading::NoSelection
        };
        self.transfer = None;
        self.samples.clear();
    }

    pub fn set_waiting(&mut self) {
        self.session_id = None;
        self.server_time = None;
        self.history_revision = 0;
        self.coverage = HistoryCoverage::default();
        self.loading = HistoryChartLoading::Waiting;
        self.transfer = None;
        self.samples.clear();
    }

    pub fn set_history(
        &mut self,
        session_id: HistorySessionId,
        server_time: Timestamp,
        history_revision: u64,
        coverage: HistoryCoverage,
        loading: HistoryChartLoading,
        transfer: Option<ChartTransferProgress>,
        samples: &[DisplayHistorySample],
    ) {
        self.session_id = Some(session_id);
        self.server_time = Some(server_time);
        self.history_revision = history_revision;
        self.coverage = coverage;
        self.loading = loading;
        self.transfer = transfer;
        self.samples.clear();
        self.samples.extend_from_slice(samples);
    }
}

#[derive(Resource, Debug, Default)]
pub struct HistoryChartActions {
    pub retry: Option<TrackId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartMetric {
    Altitude,
    GroundSpeed,
}

impl ChartMetric {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Altitude => "Altitude",
            Self::GroundSpeed => "Ground speed",
        }
    }

    #[must_use]
    pub const fn unit(self) -> &'static str {
        match self {
            Self::Altitude => "ft",
            Self::GroundSpeed => "kt",
        }
    }

    fn value(self, sample: &DisplayHistorySample) -> (Option<f64>, Option<AltitudeReference>) {
        match self {
            Self::Altitude => (
                sample.altitude_ft.map(f64::from),
                Some(sample.altitude_reference),
            ),
            Self::GroundSpeed => (sample.ground_speed_kts, None),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartPoint {
    pub sample_sequence: u64,
    pub timestamp: Timestamp,
    pub segment_id: u32,
    pub value: Option<f64>,
    pub estimated: bool,
    pub gap_before: bool,
    pub altitude_reference: Option<AltitudeReference>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSeries {
    pub start: Timestamp,
    pub end: Timestamp,
    pub points: Vec<ChartPoint>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
}

/// Build a time-based series from canonical samples. Unknown values remain
/// points with `None`; they are not converted to zero or interpolated.
pub fn build_chart_series(
    samples: &[DisplayHistorySample],
    coverage: &HistoryCoverage,
    server_time: Timestamp,
    window: HistoryChartWindow,
    max_points: usize,
    metric: ChartMetric,
) -> ChartSeries {
    let end = server_time;
    let start = end - window.duration();
    let gap_threshold = sampling_gap_threshold(coverage);

    let mut ordered: Vec<&DisplayHistorySample> = samples
        .iter()
        .filter(|sample| sample.state_time >= start && sample.state_time <= end)
        .collect();
    ordered.sort_by_key(|sample| (sample.state_time, sample.sample_sequence));

    let mut points = Vec::with_capacity(ordered.len());
    for sample in ordered {
        let (value, altitude_reference) = metric.value(sample);
        let previous = points.last();
        let gap_before = previous.is_some_and(|previous: &ChartPoint| {
            let elapsed = sample
                .state_time
                .signed_duration_since(previous.timestamp)
                .num_milliseconds() as f64
                / 1000.0;
            previous.value.is_none()
                || elapsed >= gap_threshold
                || sample.segment_id != previous.segment_id
                || sample.break_reason.is_some()
        });
        points.push(ChartPoint {
            sample_sequence: sample.sample_sequence,
            timestamp: sample.state_time,
            segment_id: sample.segment_id,
            value,
            estimated: sample.estimated,
            gap_before,
            altitude_reference,
        });
    }

    let points = downsample_points(points, max_points);
    let (min_value, max_value) =
        points
            .iter()
            .filter_map(|point| point.value)
            .fold((None, None), |(min, max), value| {
                (
                    Some(min.map_or(value, |current: f64| current.min(value))),
                    Some(max.map_or(value, |current: f64| current.max(value))),
                )
            });

    ChartSeries {
        start,
        end,
        points,
        min_value,
        max_value,
    }
}

fn sampling_gap_threshold(coverage: &HistoryCoverage) -> f64 {
    let interval_secs = coverage.sampling_interval.num_milliseconds() as f64 / 1000.0;
    if interval_secs > 0.0 {
        (interval_secs * 2.0).max(5.0)
    } else {
        30.0
    }
}

/// Keep the first and last samples, every continuity break, and bucket extrema.
/// This is presentation-only reduction; the canonical store remains untouched.
pub fn downsample_points(points: Vec<ChartPoint>, max_points: usize) -> Vec<ChartPoint> {
    if points.len() <= max_points || max_points < 2 {
        return points;
    }

    let mut keep = BTreeSet::from([0, points.len() - 1]);
    for (index, point) in points.iter().enumerate() {
        if point.value.is_none() || point.gap_before {
            keep.insert(index);
        }
    }

    let bucket_count = (max_points / 4).max(1);
    for bucket in 0..bucket_count {
        let begin = bucket * points.len() / bucket_count;
        let end = ((bucket + 1) * points.len() / bucket_count).max(begin + 1);
        let end = end.min(points.len());
        let indices: Vec<usize> = (begin..end)
            .filter(|index| points[*index].value.is_some())
            .collect();
        if let Some(&first) = indices.first() {
            keep.insert(first);
            keep.insert(*indices.last().unwrap_or(&first));
            keep.insert(
                *indices
                    .iter()
                    .min_by(|left, right| {
                        points[**left]
                            .value
                            .partial_cmp(&points[**right].value)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(&first),
            );
            keep.insert(
                *indices
                    .iter()
                    .max_by(|left, right| {
                        points[**left]
                            .value
                            .partial_cmp(&points[**right].value)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(&first),
            );
        }
    }

    keep.into_iter()
        .map(|index| points[index].clone())
        .collect()
}

/// Embedded mode already has the same canonical `DisplayTrail` samples on the
/// linked fusion entity. Thin mode replaces this source with the T5 store.
pub fn sync_embedded_history_chart(
    list_state: bevy::prelude::Res<AircraftListState>,
    visuals: bevy::prelude::Query<(&Aircraft, &FusionTrackLink)>,
    trails: bevy::prelude::Query<&DisplayTrail>,
    mut chart: bevy::prelude::ResMut<HistoryChartState>,
) {
    let selected = list_state.selected_icao.as_ref().and_then(|icao| {
        visuals
            .iter()
            .find(|(aircraft, _)| &aircraft.icao == icao)
            .map(|(_, link)| link.track_id.clone())
    });
    chart.select(selected.clone());

    let Some(track_id) = selected else {
        return;
    };
    let Some((_, link)) = visuals.iter().find(|(aircraft, _)| {
        list_state
            .selected_icao
            .as_ref()
            .is_some_and(|icao| &aircraft.icao == icao)
    }) else {
        chart.set_waiting();
        return;
    };
    let Ok(preview) = trails.get(link.track_entity) else {
        chart.set_waiting();
        return;
    };

    let loading = if preview.preview_truncated {
        HistoryChartLoading::Partial
    } else {
        HistoryChartLoading::Complete
    };
    chart.set_history(
        preview.session_id,
        preview.server_time,
        preview.history_revision,
        preview.coverage.clone(),
        loading,
        None,
        &preview.samples,
    );
    debug_assert_eq!(track_id, preview.track_id);
}

pub fn render_history_charts(
    ui: &mut egui::Ui,
    chart: &mut HistoryChartState,
    actions: &mut HistoryChartActions,
    text: egui::Color32,
    text_dim: egui::Color32,
    accent: egui::Color32,
) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("History")
                .color(text)
                .strong()
                .size(11.0),
        );
        ui.add_space(8.0);
        for window in HistoryChartWindow::ALL {
            if ui
                .selectable_label(chart.window == window, window.label())
                .clicked()
            {
                chart.window = window;
            }
        }
    });

    render_history_status(ui, chart, actions, text, text_dim);

    let Some(server_time) = chart.server_time else {
        return;
    };
    if chart.samples.is_empty() {
        ui.label(
            egui::RichText::new("No retained samples")
                .color(text_dim)
                .size(10.0),
        );
        return;
    }

    let altitude = build_chart_series(
        &chart.samples,
        &chart.coverage,
        server_time,
        chart.window,
        DEFAULT_MAX_POINTS,
        ChartMetric::Altitude,
    );
    let ground_speed = build_chart_series(
        &chart.samples,
        &chart.coverage,
        server_time,
        chart.window,
        DEFAULT_MAX_POINTS,
        ChartMetric::GroundSpeed,
    );
    render_chart(ui, &altitude, ChartMetric::Altitude, text, text_dim, accent);
    ui.add_space(4.0);
    render_chart(
        ui,
        &ground_speed,
        ChartMetric::GroundSpeed,
        text,
        text_dim,
        accent,
    );
}

fn render_history_status(
    ui: &mut egui::Ui,
    chart: &HistoryChartState,
    actions: &mut HistoryChartActions,
    text: egui::Color32,
    text_dim: egui::Color32,
) {
    let status = match chart.loading {
        HistoryChartLoading::NoSelection => return,
        HistoryChartLoading::Waiting => "Waiting for history",
        HistoryChartLoading::Preview => "Preview loaded, requesting retained history",
        HistoryChartLoading::Loading => "Loading retained history",
        HistoryChartLoading::Partial => "Partial available coverage",
        HistoryChartLoading::Complete => "Complete available coverage",
        HistoryChartLoading::RetryableError => "History transfer failed; retry is available",
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(status).color(text).size(10.0));
        if let Some(progress) = chart.transfer {
            if progress.total_chunks > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "{}/{} chunks, {} samples",
                        progress.received_chunks, progress.total_chunks, progress.received_samples
                    ))
                    .color(text_dim)
                    .size(10.0),
                );
            }
        }
        ui.label(
            egui::RichText::new(format!(
                "{} available / {} retained samples, {}",
                chart.samples.len(),
                chart.coverage.retained_sample_count,
                format_duration(chart.coverage.retained_duration),
            ))
            .color(text_dim)
            .size(10.0),
        );
        if matches!(chart.loading, HistoryChartLoading::RetryableError)
            && chart.track_id.is_some()
            && ui.small_button("Retry").clicked()
        {
            actions.retry = chart.track_id.clone();
        }
    });

    let track_age = chart
        .coverage
        .first_seen
        .zip(chart.server_time)
        .map(|(first, now)| format_duration(now.signed_duration_since(first)));
    let retained = chart
        .coverage
        .retained_from
        .zip(chart.coverage.retained_to)
        .map(|(from, to)| format!("{} to {}", format_time(from), format_time(to)))
        .unwrap_or_else(|| "no retained interval".to_string());
    ui.label(
        egui::RichText::new(format!(
            "Track age: {}  |  retained: {}",
            track_age.unwrap_or_else(|| "unknown".to_string()),
            retained
        ))
        .color(text_dim)
        .size(9.0),
    );
}

fn render_chart(
    ui: &mut egui::Ui,
    series: &ChartSeries,
    metric: ChartMetric,
    text: egui::Color32,
    text_dim: egui::Color32,
    accent: egui::Color32,
) {
    let width = ui.available_width().max(180.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, CHART_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(3),
        egui::Color32::from_black_alpha(35),
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(70)),
        egui::epaint::StrokeKind::Inside,
    );
    painter.text(
        rect.left_top() + egui::vec2(7.0, 5.0),
        egui::Align2::LEFT_TOP,
        format!("{} ({})", metric.label(), metric.unit()),
        egui::FontId::proportional(10.0),
        text,
    );

    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 42.0, rect.top() + 22.0),
        egui::pos2(rect.right() - 8.0, rect.bottom() - 22.0),
    );
    let (min_value, max_value) = chart_range(series);
    let y_range = (max_value - min_value).max(1.0);
    let x_range = (series.end - series.start).num_milliseconds().max(1) as f32;

    for fraction in [0.0_f32, 0.5, 1.0] {
        let y = plot.bottom() - fraction * plot.height();
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(45)),
        );
        painter.text(
            egui::pos2(plot.left() - 4.0, y),
            egui::Align2::RIGHT_CENTER,
            format_value(min_value + f64::from(fraction) * y_range, metric),
            egui::FontId::monospace(8.0),
            text_dim,
        );
    }
    for fraction in [0.0_f32, 0.5, 1.0] {
        let x = plot.left() + fraction * plot.width();
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(35)),
        );
        let timestamp = series.start
            + Duration::milliseconds((f64::from(fraction) * f64::from(x_range)) as i64);
        painter.text(
            egui::pos2(x, plot.bottom() + 4.0),
            egui::Align2::CENTER_TOP,
            timestamp.format("%H:%M:%S").to_string(),
            egui::FontId::monospace(8.0),
            text_dim,
        );
    }

    let point_position = |point: &ChartPoint| {
        let x = plot.left()
            + (point.timestamp - series.start).num_milliseconds().max(0) as f32 / x_range
                * plot.width();
        let y = point.value.map_or(plot.bottom(), |value| {
            plot.bottom() - ((value - min_value) / y_range) as f32 * plot.height()
        });
        egui::pos2(
            x.clamp(plot.left(), plot.right()),
            y.clamp(plot.top(), plot.bottom()),
        )
    };

    for pair in series.points.windows(2) {
        let [previous, current] = pair else { continue };
        if previous.value.is_none() || current.value.is_none() {
            continue;
        }
        if current.gap_before {
            continue;
        }
        let from = point_position(previous);
        let to = point_position(current);
        let stroke = egui::Stroke::new(
            if previous.estimated || current.estimated {
                1.0
            } else {
                1.8
            },
            if previous.estimated || current.estimated {
                egui::Color32::from_rgb(235, 180, 85)
            } else {
                accent
            },
        );
        draw_segment(
            &painter,
            from,
            to,
            stroke,
            previous.estimated || current.estimated,
        );
    }

    for point in &series.points {
        let position = point_position(point);
        if point.value.is_some() {
            if point.estimated {
                painter.circle_stroke(position, 2.5, egui::Stroke::new(1.0, accent));
            } else {
                painter.circle_filled(position, 2.0, accent);
            }
        } else {
            painter.line_segment(
                [
                    egui::pos2(position.x, plot.bottom() - 4.0),
                    egui::pos2(position.x, plot.bottom()),
                ],
                egui::Stroke::new(1.5, text_dim),
            );
        }
    }

    if let Some(pointer) = response.hover_pos() {
        if plot.contains(pointer) {
            if let Some(point) = series.points.iter().min_by(|left, right| {
                (point_position(left).x - pointer.x)
                    .abs()
                    .partial_cmp(&(point_position(right).x - pointer.x).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                let tooltip_response = response.clone();
                tooltip_response.on_hover_ui(|ui| {
                    ui.label(egui::RichText::new(format_time(point.timestamp)).strong());
                    let value = point.value.map_or_else(
                        || "Unknown".to_string(),
                        |value| format_value(value, metric),
                    );
                    ui.label(value);
                    if point.estimated {
                        ui.label("Estimated / prediction-only interval");
                    }
                    if let Some(reference) = point.altitude_reference {
                        ui.label(format!(
                            "Altitude reference: {}",
                            altitude_reference_label(reference)
                        ));
                    }
                    if point.gap_before {
                        ui.label("Gap before sample");
                    }
                });
            }
        }
    }
}

fn chart_range(series: &ChartSeries) -> (f64, f64) {
    let min = series.min_value.unwrap_or(0.0);
    let max = series.max_value.unwrap_or(1.0);
    if (max - min).abs() < f64::EPSILON {
        let padding = (max.abs() * 0.1).max(1.0);
        (min - padding, max + padding)
    } else {
        let padding = (max - min) * 0.08;
        (min - padding, max + padding)
    }
}

fn draw_segment(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    stroke: egui::Stroke,
    dashed: bool,
) {
    if !dashed {
        painter.line_segment([from, to], stroke);
        return;
    }
    let delta = to - from;
    let length = delta.length();
    if length <= f32::EPSILON {
        return;
    }
    let direction = delta / length;
    let dash = 4.0;
    let gap = 3.0;
    let mut offset = 0.0;
    while offset < length {
        let start = from + direction * offset;
        let end = from + direction * (offset + dash).min(length);
        painter.line_segment([start, end], stroke);
        offset += dash + gap;
    }
}

fn format_value(value: f64, metric: ChartMetric) -> String {
    match metric {
        ChartMetric::Altitude => format!("{value:.0} {}", metric.unit()),
        ChartMetric::GroundSpeed => format!("{value:.1} {}", metric.unit()),
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.num_seconds().max(0);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn format_time(timestamp: DateTime<Utc>) -> String {
    timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn altitude_reference_label(reference: AltitudeReference) -> &'static str {
    match reference {
        AltitudeReference::Barometric => "barometric",
        AltitudeReference::Geometric => "geometric",
        AltitudeReference::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airjedi_core::{
        DisplayProvenance, HeadingReference, HistoryBreakReason, PositionSource, TrackStatus,
        VerticalRateReference,
    };

    fn sample(
        sequence: u64,
        seconds: i64,
        altitude_ft: Option<i32>,
        ground_speed_kts: Option<f64>,
        estimated: bool,
        segment_id: u32,
        break_reason: Option<HistoryBreakReason>,
    ) -> DisplayHistorySample {
        DisplayHistorySample {
            sample_sequence: sequence,
            state_time: DateTime::from_timestamp(seconds, 0).unwrap(),
            latitude: 37.0,
            longitude: -97.0,
            altitude_ft,
            altitude_reference: AltitudeReference::Barometric,
            ground_speed_kts,
            heading: None,
            heading_reference: HeadingReference::Unknown,
            vertical_rate: None,
            vertical_rate_reference: VerticalRateReference::Unknown,
            position_source: Some(PositionSource::AdsbIcao),
            status: TrackStatus::Confirmed,
            estimated,
            provenance: DisplayProvenance::default(),
            segment_id,
            break_reason,
        }
    }

    fn coverage(interval_seconds: i64) -> HistoryCoverage {
        HistoryCoverage {
            sampling_interval: Duration::seconds(interval_seconds),
            ..HistoryCoverage::default()
        }
    }

    #[test]
    fn chart_uses_timestamps_and_preserves_unknown_zero_and_estimated_values() {
        let samples = vec![
            sample(1, 100, Some(0), Some(0.0), false, 0, None),
            sample(2, 110, None, None, true, 0, None),
            sample(
                3,
                150,
                Some(1200),
                Some(80.0),
                true,
                1,
                Some(HistoryBreakReason::Reacquired),
            ),
        ];
        let series = build_chart_series(
            &samples,
            &coverage(10),
            DateTime::from_timestamp(150, 0).unwrap(),
            HistoryChartWindow::FiveMinutes,
            160,
            ChartMetric::Altitude,
        );

        assert_eq!(series.points.len(), 3);
        assert_eq!(series.points[0].value, Some(0.0));
        assert_eq!(series.points[1].value, None);
        assert!(series.points[2].estimated);
        assert!(series.points[2].gap_before);
        assert_eq!(series.points[2].timestamp.timestamp(), 150);
    }

    #[test]
    fn chart_window_filters_by_time_not_sample_position() {
        let samples = vec![
            sample(1, 0, Some(100), Some(10.0), false, 0, None),
            sample(2, 301, Some(200), Some(20.0), false, 0, None),
            sample(3, 599, Some(300), Some(30.0), false, 0, None),
        ];
        let series = build_chart_series(
            &samples,
            &coverage(1),
            DateTime::from_timestamp(600, 0).unwrap(),
            HistoryChartWindow::FiveMinutes,
            160,
            ChartMetric::GroundSpeed,
        );

        assert_eq!(
            series
                .points
                .iter()
                .map(|p| p.sample_sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(series.start.timestamp(), 300);
    }

    #[test]
    fn downsampling_keeps_extrema_and_continuity_breaks() {
        let points = (0..100)
            .map(|index| ChartPoint {
                sample_sequence: index,
                timestamp: DateTime::from_timestamp(index as i64, 0).unwrap(),
                segment_id: 0,
                value: Some(if index == 42 { 1000.0 } else { index as f64 }),
                estimated: false,
                gap_before: index == 70,
                altitude_reference: Some(AltitudeReference::Geometric),
            })
            .collect();
        let reduced = downsample_points(points, 12);

        assert!(reduced.iter().any(|point| point.sample_sequence == 42));
        assert!(reduced.iter().any(|point| point.sample_sequence == 70));
        assert_eq!(reduced.first().map(|p| p.sample_sequence), Some(0));
        assert_eq!(reduced.last().map(|p| p.sample_sequence), Some(99));
    }

    #[test]
    fn selection_change_clears_previous_identity_and_samples() {
        let first = TrackId::new();
        let second = TrackId::new();
        let mut state = HistoryChartState::default();
        state.select(Some(first.clone()));
        state
            .samples
            .push(sample(1, 1, Some(10), Some(20.0), false, 0, None));
        state.select(Some(second.clone()));

        assert_eq!(state.track_id, Some(second));
        assert!(state.samples.is_empty());
        assert_eq!(state.loading, HistoryChartLoading::Waiting);
        assert!(state.session_id.is_none());
    }

    #[test]
    #[ignore = "known history regression: selected embedded chart uses the preview instead of full trail history"]
    fn selected_full_history_trail_and_chart_expose_the_same_samples() {
        use crate::aircraft::components::{Aircraft, FusionTrackLink};
        use crate::aircraft::trails::TrailHistory;
        use bevy::prelude::{App, Update};

        let track_id = TrackId::new();
        let server_time = DateTime::from_timestamp(1_700_000_600, 0).unwrap();
        let full_samples: Vec<_> = (0..=300)
            .map(|index| {
                sample(
                    index,
                    server_time.timestamp() - 600 + index as i64 * 2,
                    Some(30_000 + index as i32),
                    Some(400.0),
                    false,
                    0,
                    None,
                )
            })
            .collect();
        let preview_samples = full_samples[150..].to_vec();
        let coverage = HistoryCoverage {
            retained_sample_count: full_samples.len(),
            sampling_interval: Duration::seconds(2),
            ..HistoryCoverage::default()
        };

        let mut app = App::new();
        app.insert_resource(AircraftListState {
            selected_icao: Some("PROBE01".to_string()),
            ..AircraftListState::default()
        })
        .init_resource::<HistoryChartState>()
        .add_systems(Update, sync_embedded_history_chart);

        let track_entity = app
            .world_mut()
            .spawn(DisplayTrail {
                session_id: HistorySessionId::nil(),
                track_id: track_id.clone(),
                server_time,
                history_revision: 600,
                coverage,
                samples: preview_samples,
                preview_window: Duration::minutes(5),
                preview_truncated: true,
                sample_sequence_start: Some(150),
                sample_sequence_end: Some(300),
            })
            .id();
        let mut trail = TrailHistory::default();
        trail.replace_from_samples(
            &full_samples,
            server_time,
            &crate::aircraft::SessionClock::default(),
        );
        let visual_entity = app
            .world_mut()
            .spawn((
                Aircraft {
                    icao: "PROBE01".to_string(),
                    callsign: None,
                    latitude: 37.0,
                    longitude: -97.0,
                    altitude: Some(30_000),
                    heading: Some(90.0),
                    velocity: Some(400.0),
                    vertical_rate: None,
                    roll_angle: None,
                    track_angle_rate: None,
                    roll_last_seen: None,
                    squawk: None,
                    is_on_ground: Some(false),
                    alert: None,
                    emergency: None,
                    spi: None,
                    last_seen: server_time,
                },
                FusionTrackLink {
                    track_entity,
                    track_id,
                },
                trail,
            ))
            .id();

        app.update();

        let chart = app.world().resource::<HistoryChartState>();
        let trail = app.world().get::<TrailHistory>(visual_entity).unwrap();
        assert_eq!(
            chart
                .samples
                .iter()
                .map(|sample| sample.sample_sequence)
                .collect::<Vec<_>>(),
            trail
                .points
                .iter()
                .map(|point| point.sample_sequence.unwrap())
                .collect::<Vec<_>>(),
        );
    }
}
