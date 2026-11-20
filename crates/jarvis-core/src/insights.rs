use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

const MAX_DAYS: usize = 90;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayInsight {
    pub day: String,
    pub commands: u64,
    pub failed: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Insights {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub rules: u64,
    pub intent: u64,
    pub ollama: u64,
    pub local: u64,
    pub days: Vec<DayInsight>,
}

impl Insights {
    pub fn load(path: &Path) -> Self {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save_atomic(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, path)
    }

    pub fn record(&mut self, source: &str, ok: bool) {
        let day = today_key();
        self.record_on(&day, source, ok);
    }

    pub fn record_on(&mut self, day: &str, source: &str, ok: bool) {
        self.total += 1;
        if ok {
            self.succeeded += 1;
        } else {
            self.failed += 1;
        }
        match source {
            "Rules" => self.rules += 1,
            "Intent model" => self.intent += 1,
            "Ollama" => self.ollama += 1,
            _ => self.local += 1,
        }
        if let Some(bucket) = self.days.iter_mut().find(|bucket| bucket.day == day) {
            bucket.commands += 1;
            if !ok {
                bucket.failed += 1;
            }
        } else {
            self.days.push(DayInsight {
                day: day.to_string(),
                commands: 1,
                failed: u64::from(!ok),
            });
        }
        self.days.sort_by(|a, b| a.day.cmp(&b.day));
        if self.days.len() > MAX_DAYS {
            self.days.drain(..self.days.len() - MAX_DAYS);
        }
    }

    pub fn success_rate(&self) -> Option<u32> {
        (self.total > 0).then(|| ((self.succeeded * 100) / self.total) as u32)
    }

    pub fn last_days(&self, count: usize) -> Vec<DayInsight> {
        let today = Local::now().date_naive();
        let by_day: BTreeMap<&str, &DayInsight> = self
            .days
            .iter()
            .map(|bucket| (bucket.day.as_str(), bucket))
            .collect();
        (0..count)
            .rev()
            .map(|days_ago| {
                let day = day_key(today - Duration::days(days_ago as i64));
                by_day.get(day.as_str()).map_or_else(
                    || DayInsight {
                        day,
                        commands: 0,
                        failed: 0,
                    },
                    |bucket| (*bucket).clone(),
                )
            })
            .collect()
    }

    pub fn branch_total(&self) -> u64 {
        self.rules + self.intent + self.ollama + self.local
    }
}

pub fn today_key() -> String {
    day_key(Local::now().date_naive())
}

pub fn day_key(date: NaiveDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
}

pub fn day_key_ago(days_ago: i64) -> String {
    day_key(Local::now().date_naive() - Duration::days(days_ago))
}

pub fn weekday_short(day: &str) -> String {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|date| date.format("%a").to_string())
        .unwrap_or_else(|_| "?".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_branches_and_success() {
        let mut insights = Insights::default();
        insights.record_on("2026-11-19", "Rules", true);
        insights.record_on("2026-11-19", "Intent model", true);
        insights.record_on("2026-11-20", "Ollama", false);
        insights.record_on("2026-11-20", "Local", false);
        assert_eq!(insights.total, 4);
        assert_eq!(insights.succeeded, 2);
        assert_eq!(insights.failed, 2);
        assert_eq!(insights.success_rate(), Some(50));
        assert_eq!(insights.rules, 1);
        assert_eq!(insights.intent, 1);
        assert_eq!(insights.ollama, 1);
        assert_eq!(insights.local, 1);
        assert_eq!(insights.days[0].commands, 2);
        assert_eq!(insights.days[1].failed, 2);
    }

    #[test]
    fn last_days_fills_gaps_and_stays_ordered() {
        let mut insights = Insights::default();
        let today = today_key();
        insights.record_on(&today, "Rules", true);
        let days = insights.last_days(7);
        assert_eq!(days.len(), 7);
        assert_eq!(days.as_slice().last().unwrap().day, today);
        assert_eq!(days.as_slice().last().unwrap().commands, 1);
        assert!(days.windows(2).all(|pair| pair[0].day < pair[1].day));
        assert!(weekday_short(&today).len() >= 2);
    }

    #[test]
    fn persists_and_caps_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("insights.json");
        let mut insights = Insights::default();
        for index in 0..120 {
            insights.record_on(&format!("2025-01-{:02}", index % 28 + 1), "Rules", true);
        }
        assert!(insights.days.len() <= MAX_DAYS);
        insights.save_atomic(&path).unwrap();
        assert_eq!(Insights::load(&path), insights);
    }
}
