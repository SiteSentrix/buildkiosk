//! Daily logs: one record per project per day of what happened on site.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::project::Project;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum LogStatus {
        #[default]
        Draft = ("draft", "Draft"),
        Submitted = ("submitted", "Submitted"),
    }
}

choice! {
    #[derive(Default)]
    pub enum Sky {
        #[default]
        Clear = ("clear", "Clear"),
        PartlyCloudy = ("partly_cloudy", "Partly cloudy"),
        Overcast = ("overcast", "Overcast"),
        Rain = ("rain", "Rain"),
        Snow = ("snow", "Snow"),
        Fog = ("fog", "Fog"),
        Windy = ("windy", "Windy"),
        Storm = ("storm", "Storm"),
    }
}

/// The weather on site that day.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Weather {
    #[serde(default)]
    pub sky: Sky,
    /// Whole degrees, in whatever unit the site uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<i32>,
    #[serde(default)]
    pub notes: String,
}

/// A crew on site: who, how many and for how long.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manpower {
    pub company: String,
    #[serde(default)]
    pub trade: String,
    pub workers: u32,
    /// Hours worked by each worker, in tenths of an hour (75 = 7.5 hours).
    pub hours_tenths: u32,
    #[serde(default)]
    pub notes: String,
}

impl Manpower {
    /// Total worker-hours in tenths.
    pub fn total_tenths(&self) -> u64 {
        u64::from(self.workers).saturating_mul(u64::from(self.hours_tenths))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EquipmentUse {
    pub name: String,
    /// Tenths of an hour in use.
    pub hours_tenths: u32,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub description: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Visitor {
    pub name: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub purpose: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DailyLog {
    pub id: Id<DailyLog>,
    pub project: Id<Project>,
    pub date: Date,
    #[serde(default)]
    pub status: LogStatus,
    #[serde(default)]
    pub weather: Weather,
    #[serde(default)]
    pub work_performed: String,
    #[serde(default)]
    pub delays: String,
    #[serde(default)]
    pub safety: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub manpower: Vec<Manpower>,
    #[serde(default)]
    pub equipment: Vec<EquipmentUse>,
    #[serde(default)]
    pub deliveries: Vec<Delivery>,
    #[serde(default)]
    pub visitors: Vec<Visitor>,
    /// Photos and other files for the day.
    #[serde(default)]
    pub attachments: Vec<crate::common::FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(DailyLog, "daily_log", trash, project);

impl DailyLog {
    pub fn label(&self) -> String {
        format!("Daily log for {}", self.date.pretty())
    }

    /// Workers on site and total worker-hours (tenths).
    pub fn headcount(&self) -> (u64, u64) {
        self.manpower.iter().fold((0, 0), |(w, h), m| (w.saturating_add(u64::from(m.workers)), h.saturating_add(m.total_tenths())))
    }
}

/// "7.5" for 75 tenths.
pub fn hours_text(tenths: u64) -> String {
    if tenths.is_multiple_of(10) { format!("{}", tenths / 10) } else { format!("{}.{}", tenths / 10, tenths % 10) }
}

/// Reads "7", "7.5" or "7,5" as tenths of an hour; at most 24 hours.
pub fn parse_hours(s: &str) -> Option<u32> {
    let s = s.trim().replace(',', ".");
    let (whole, frac) = s.split_once('.').unwrap_or((s.as_str(), ""));
    if whole.is_empty() && frac.is_empty()
        || frac.len() > 1
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let w: u32 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let f: u32 = if frac.is_empty() { 0 } else { frac.parse().ok()? };
    let t = w.checked_mul(10)?.checked_add(f)?;
    (t <= 240).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours() {
        assert_eq!(parse_hours("7.5"), Some(75));
        assert_eq!(parse_hours("8"), Some(80));
        assert_eq!(parse_hours(" 0,5 "), Some(5));
        assert_eq!(parse_hours(".5"), Some(5));
        for bad in ["", ".", "-1", "7.55", "25", "abc", "1.2.3"] {
            assert_eq!(parse_hours(bad), None, "{bad}");
        }
        assert_eq!(hours_text(75), "7.5");
        assert_eq!(hours_text(80), "8");
    }
}
