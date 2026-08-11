//! Pure history aggregation over timestamped check-result state markers.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum HistoryRange {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl HistoryRange {
    pub fn duration(self) -> Duration {
        match self {
            Self::Day => Duration::hours(24),
            Self::Week => Duration::days(7),
            Self::Month => Duration::days(30),
        }
    }

    fn bucket_count(self) -> usize {
        match self {
            Self::Day => 0,
            Self::Week | Self::Month => 500,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HistoryEvent {
    pub checked_at: DateTime<Utc>,
    pub status: HistoryStatus,
    pub response_time_ms: Option<i64>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryStatus {
    Up,
    Down,
    Gap,
}

impl HistoryStatus {
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "gap" => Some(Self::Gap),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PointStatus {
    Up,
    Down,
    Gap,
    Mixed,
}

impl From<HistoryStatus> for PointStatus {
    fn from(value: HistoryStatus) -> Self {
        match value {
            HistoryStatus::Up => Self::Up,
            HistoryStatus::Down => Self::Down,
            HistoryStatus::Gap => Self::Gap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorStatus {
    NotYetChecked,
    Up,
    Down,
}

impl MonitorStatus {
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "not_yet_checked" => Some(Self::NotYetChecked),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UptimeStats {
    pub uptime_24h: Option<f64>,
    pub uptime_7d: Option<f64>,
    pub uptime_30d: Option<f64>,
    pub avg_response_time_ms_24h: Option<f64>,
    pub last_check_at: Option<String>,
    pub current_status: MonitorStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub started_at: String,
    pub ended_at: String,
    pub status: PointStatus,
    pub avg_response_time_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Incident {
    pub started_at: String,
    pub ended_at: Option<String>,
    pub duration_seconds: i64,
    pub failure_reason: Option<String>,
    pub ongoing: bool,
    pub includes_gap: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryResponse {
    pub points: Vec<HistoryPoint>,
    pub incidents: Vec<Incident>,
}

#[derive(Debug, Clone)]
struct Segment {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    status: HistoryStatus,
    response_time_ms: Option<i64>,
}

/// Uptime is duration-weighted. A `gap` marker starts unknown time that is
/// excluded until the next real result; missing history is excluded likewise.
pub fn uptime_stats(
    events: &[HistoryEvent],
    current_status: MonitorStatus,
    last_check_at: Option<String>,
    now: DateTime<Utc>,
) -> UptimeStats {
    let percentage = |duration| uptime_percent(&build_segments(events, now - duration, now));
    let day_start = now - Duration::hours(24);
    let response_times: Vec<i64> = events
        .iter()
        .filter(|event| event.checked_at >= day_start && event.checked_at < now)
        .filter_map(|event| event.response_time_ms)
        .collect();
    let avg_response_time_ms_24h = (!response_times.is_empty())
        .then(|| response_times.iter().sum::<i64>() as f64 / response_times.len() as f64);

    UptimeStats {
        uptime_24h: percentage(Duration::hours(24)),
        uptime_7d: percentage(Duration::days(7)),
        uptime_30d: percentage(Duration::days(30)),
        avg_response_time_ms_24h,
        last_check_at,
        current_status,
    }
}

pub fn history(
    events: &[HistoryEvent],
    current_status: MonitorStatus,
    range: HistoryRange,
    now: DateTime<Utc>,
) -> HistoryResponse {
    let start = now - range.duration();
    let segments = build_segments(events, start, now);
    let points = if range.bucket_count() == 0 {
        raw_points(&segments)
    } else {
        bucketed_points(&segments, events, start, now, range.bucket_count())
    };
    let mut incidents = build_incidents(events, current_status, now);
    incidents.retain(|incident| {
        incident.ongoing
            || incident
                .ended_at
                .as_deref()
                .and_then(parse_timestamp)
                .is_some_and(|ended| ended >= start)
    });
    incidents.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    HistoryResponse { points, incidents }
}

fn build_segments(
    events: &[HistoryEvent],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<Segment> {
    if start >= end {
        return Vec::new();
    }
    let mut current_status = HistoryStatus::Gap;
    let mut current_response = None;
    let mut cursor = start;

    if let Some(previous) = events.iter().rev().find(|event| event.checked_at <= start) {
        current_status = previous.status;
        current_response = previous.response_time_ms;
    }

    let mut segments = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.checked_at > start && event.checked_at < end)
    {
        if event.checked_at > cursor {
            segments.push(Segment {
                started_at: cursor,
                ended_at: event.checked_at,
                status: current_status,
                response_time_ms: current_response,
            });
        }
        cursor = event.checked_at;
        current_status = event.status;
        current_response = event.response_time_ms;
    }
    if cursor < end {
        segments.push(Segment {
            started_at: cursor,
            ended_at: end,
            status: current_status,
            response_time_ms: current_response,
        });
    }
    segments
}

fn uptime_percent(segments: &[Segment]) -> Option<f64> {
    let mut up_ms = 0_i64;
    let mut observed_ms = 0_i64;
    for segment in segments {
        let duration = (segment.ended_at - segment.started_at).num_milliseconds();
        match segment.status {
            HistoryStatus::Up => {
                up_ms += duration;
                observed_ms += duration;
            }
            HistoryStatus::Down => observed_ms += duration,
            HistoryStatus::Gap => {}
        }
    }
    (observed_ms > 0).then(|| round_one(up_ms as f64 * 100.0 / observed_ms as f64))
}

fn raw_points(segments: &[Segment]) -> Vec<HistoryPoint> {
    segments
        .iter()
        .map(|segment| HistoryPoint {
            started_at: format_timestamp(segment.started_at),
            ended_at: format_timestamp(segment.ended_at),
            status: segment.status.into(),
            avg_response_time_ms: segment.response_time_ms.map(|value| value as f64),
        })
        .collect()
}

fn bucketed_points(
    segments: &[Segment],
    events: &[HistoryEvent],
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    target_count: usize,
) -> Vec<HistoryPoint> {
    let total_ms = (end - start).num_milliseconds();
    let bucket_ms = (total_ms + target_count as i64 - 1) / target_count as i64;
    let mut points = Vec::with_capacity(target_count);
    let mut segment_index = 0;
    let mut event_index = events.partition_point(|event| event.checked_at < start);

    for index in 0..target_count {
        let bucket_start = start + Duration::milliseconds(bucket_ms * index as i64);
        if bucket_start >= end {
            break;
        }
        let bucket_end = (bucket_start + Duration::milliseconds(bucket_ms)).min(end);
        while segment_index < segments.len() && segments[segment_index].ended_at <= bucket_start {
            segment_index += 1;
        }
        let statuses: BTreeSet<HistoryStatus> = segments[segment_index..]
            .iter()
            .take_while(|segment| segment.started_at < bucket_end)
            .filter(|segment| segment.started_at < bucket_end && segment.ended_at > bucket_start)
            .map(|segment| segment.status)
            .collect();
        let status = if statuses.contains(&HistoryStatus::Gap) {
            PointStatus::Gap
        } else if statuses.len() > 1 {
            PointStatus::Mixed
        } else {
            statuses
                .first()
                .copied()
                .unwrap_or(HistoryStatus::Gap)
                .into()
        };
        while event_index < events.len() && events[event_index].checked_at < bucket_start {
            event_index += 1;
        }
        let mut response_total = 0_i64;
        let mut response_count = 0_i64;
        while event_index < events.len() && events[event_index].checked_at < bucket_end {
            if let Some(response_time) = events[event_index].response_time_ms {
                response_total += response_time;
                response_count += 1;
            }
            event_index += 1;
        }
        let average = (status != PointStatus::Gap && response_count > 0)
            .then(|| response_total as f64 / response_count as f64);
        points.push(HistoryPoint {
            started_at: format_timestamp(bucket_start),
            ended_at: format_timestamp(bucket_end),
            status,
            avg_response_time_ms: average,
        });
    }
    points
}

struct OpenIncident {
    started_at: DateTime<Utc>,
    running_from: Option<DateTime<Utc>>,
    last_observed_down_at: DateTime<Utc>,
    duration_seconds: i64,
    failure_reason: Option<String>,
    includes_gap: bool,
}

fn build_incidents(
    events: &[HistoryEvent],
    current_status: MonitorStatus,
    now: DateTime<Utc>,
) -> Vec<Incident> {
    let mut incidents = Vec::new();
    let mut open: Option<OpenIncident> = None;

    for event in events.iter().filter(|event| event.checked_at <= now) {
        match event.status {
            HistoryStatus::Down => match &mut open {
                Some(incident) => {
                    incident.last_observed_down_at = event.checked_at;
                    if incident.running_from.is_none() {
                        incident.running_from = Some(event.checked_at);
                    }
                }
                None => {
                    open = Some(OpenIncident {
                        started_at: event.checked_at,
                        running_from: Some(event.checked_at),
                        last_observed_down_at: event.checked_at,
                        duration_seconds: 0,
                        failure_reason: event.failure_reason.clone(),
                        includes_gap: false,
                    });
                }
            },
            HistoryStatus::Gap => {
                if let Some(incident) = &mut open {
                    if let Some(running_from) = incident.running_from.take() {
                        incident.duration_seconds +=
                            (event.checked_at - running_from).num_seconds().max(0);
                    }
                    incident.last_observed_down_at = event.checked_at;
                    incident.includes_gap = true;
                }
            }
            HistoryStatus::Up => {
                if let Some(mut incident) = open.take() {
                    let ended_at = if let Some(running_from) = incident.running_from.take() {
                        incident.duration_seconds +=
                            (event.checked_at - running_from).num_seconds().max(0);
                        event.checked_at
                    } else {
                        incident.last_observed_down_at
                    };
                    incidents.push(finish_incident(incident, Some(ended_at), false));
                }
            }
        }
    }

    if let Some(mut incident) = open {
        let ongoing = current_status == MonitorStatus::Down;
        let ended_at = if ongoing {
            if let Some(running_from) = incident.running_from.take() {
                incident.duration_seconds += (now - running_from).num_seconds().max(0);
            }
            None
        } else {
            Some(incident.last_observed_down_at)
        };
        incidents.push(finish_incident(incident, ended_at, ongoing));
    }
    incidents
}

fn finish_incident(
    incident: OpenIncident,
    ended_at: Option<DateTime<Utc>>,
    ongoing: bool,
) -> Incident {
    Incident {
        started_at: format_timestamp(incident.started_at),
        ended_at: ended_at.map(format_timestamp),
        duration_seconds: incident.duration_seconds,
        failure_reason: incident.failure_reason,
        ongoing,
        includes_gap: incident.includes_gap,
    }
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

pub fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    fn event(at: DateTime<Utc>, status: &str) -> HistoryEvent {
        HistoryEvent {
            checked_at: at,
            status: HistoryStatus::from_db(status).unwrap(),
            response_time_ms: (status != "gap").then_some(100),
            failure_reason: (status == "down").then(|| "timeout".into()),
        }
    }

    #[test]
    fn uptime_excludes_gap_duration() {
        let start = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let end = start + Duration::hours(24);
        let events = vec![
            event(start, "up"),
            event(start + Duration::hours(12), "down"),
            event(start + Duration::hours(18), "gap"),
        ];

        assert_eq!(
            uptime_percent(&build_segments(&events, start, end)),
            Some(66.7)
        );
        assert_eq!(uptime_percent(&build_segments(&[], start, end)), None);
    }

    #[test]
    fn bucket_count_stays_bounded_and_marks_mixed_buckets() {
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = start + Duration::days(7);
        let events = vec![
            event(start, "up"),
            event(start + Duration::minutes(5), "down"),
        ];
        let segments = build_segments(&events, start, end);
        let points = bucketed_points(&segments, &events, start, end, 500);

        assert!(points.len() <= 500);
        assert_eq!(points[0].status, PointStatus::Mixed);
    }

    #[test]
    fn bucket_boundaries_assign_events_to_the_following_bucket() {
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = start + Duration::hours(2);
        let events = vec![
            event(start, "up"),
            event(start + Duration::hours(1), "down"),
        ];

        let points = bucketed_points(&build_segments(&events, start, end), &events, start, end, 2);

        assert_eq!(points.len(), 2);
        assert_eq!(points[0].status, PointStatus::Up);
        assert_eq!(points[1].status, PointStatus::Down);
    }

    #[test]
    fn buckets_containing_unknown_time_are_gray_and_break_latency_lines() {
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        let end = start + Duration::hours(2);
        let events = vec![
            event(start, "up"),
            event(start + Duration::minutes(30), "gap"),
            event(start + Duration::hours(1), "up"),
        ];

        let points = bucketed_points(&build_segments(&events, start, end), &events, start, end, 2);

        assert_eq!(points[0].status, PointStatus::Gap);
        assert_eq!(points[0].avg_response_time_ms, None);
        assert_eq!(points[1].status, PointStatus::Up);
    }

    #[test]
    fn incidents_cover_multiple_ongoing_and_gap_interrupted_outages() {
        let start = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let events = vec![
            event(start, "down"),
            event(start + Duration::hours(1), "up"),
            event(start + Duration::hours(2), "down"),
            event(start + Duration::hours(3), "gap"),
            event(start + Duration::hours(5), "down"),
        ];

        let incidents = build_incidents(&events, MonitorStatus::Down, start + Duration::hours(6));

        assert_eq!(incidents.len(), 2);
        assert_eq!(incidents[0].duration_seconds, 3600);
        assert!(!incidents[0].ongoing);
        assert_eq!(incidents[1].duration_seconds, 7200);
        assert!(incidents[1].ongoing);
        assert!(incidents[1].includes_gap);
    }

    #[test]
    fn down_gap_up_ends_at_the_gap_and_excludes_unknown_time() {
        let start = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let events = vec![
            event(start, "down"),
            event(start + Duration::hours(1), "gap"),
            event(start + Duration::hours(4), "up"),
        ];

        let incidents = build_incidents(&events, MonitorStatus::Up, start + Duration::hours(5));

        assert_eq!(incidents[0].duration_seconds, 3600);
        assert_eq!(
            incidents[0].ended_at,
            Some(format_timestamp(start + Duration::hours(1)))
        );
        assert!(incidents[0].includes_gap);
    }
}
