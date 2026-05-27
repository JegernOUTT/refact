use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use chrono::{TimeZone, Timelike, Utc};
use chrono_tz::Tz;

use super::cron_expr::next_run_ms;

pub struct JitterConfig {
    pub recurring_frac: f64,
    pub recurring_cap_ms: u64,
    pub one_shot_max_ms: u64,
    pub one_shot_floor_ms: u64,
    pub one_shot_minute_mod: u32,
}

impl Default for JitterConfig {
    fn default() -> Self {
        Self {
            recurring_frac: 0.10,
            recurring_cap_ms: 15 * 60 * 1000,
            one_shot_max_ms: 90_000,
            one_shot_floor_ms: 0,
            one_shot_minute_mod: 30,
        }
    }
}

pub fn jitter_frac(task_id: &str) -> f64 {
    let mut hasher = DefaultHasher::new();
    task_id.hash(&mut hasher);
    let mantissa = hasher.finish() >> 11;
    mantissa as f64 / (1_u64 << 53) as f64
}

pub fn jittered_next_run_ms(
    expr: &str,
    from_ms: u64,
    task_id: &str,
    cfg: &JitterConfig,
    tz: Tz,
) -> Option<u64> {
    let t1 = next_run_ms(expr, from_ms, tz)?;
    let Some(t2) = next_run_ms(expr, t1, tz) else {
        return Some(t1);
    };
    let gap = t2 - t1;
    let jitter = (jitter_frac(task_id) * cfg.recurring_frac * gap as f64)
        .min(cfg.recurring_cap_ms as f64) as u64;
    Some(t1 + jitter)
}

pub fn one_shot_jittered_next_run_ms(
    expr: &str,
    from_ms: u64,
    task_id: &str,
    cfg: &JitterConfig,
    tz: Tz,
) -> Option<u64> {
    let t1 = next_run_ms(expr, from_ms, tz)?;
    if local_minute(t1, tz)? % cfg.one_shot_minute_mod != 0 {
        return Some(t1);
    }
    let range_ms = cfg.one_shot_max_ms - cfg.one_shot_floor_ms;
    let lead = cfg.one_shot_floor_ms + (jitter_frac(task_id) * range_ms as f64) as u64;
    Some(t1.saturating_sub(lead).max(from_ms))
}

fn local_minute(ms: u64, tz: Tz) -> Option<u32> {
    let ms = i64::try_from(ms).ok()?;
    Some(
        Utc.timestamp_millis_opt(ms)
            .single()?
            .with_timezone(&tz)
            .minute(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> u64 {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    }

    #[test]
    fn same_task_id_same_jitter_frac() {
        let first = jitter_frac("task-a");
        let second = jitter_frac("task-a");

        assert_eq!(first, second);
        assert!((0.0..1.0).contains(&first));
    }

    #[test]
    fn recurring_jitter_bounded_by_frac_and_cap() {
        let from = utc_ms(2026, 1, 1, 0, 0);
        let base = next_run_ms("*/5 * * * *", from, chrono_tz::UTC).unwrap();
        let cfg = JitterConfig {
            recurring_cap_ms: 1_000,
            ..JitterConfig::default()
        };
        let jittered =
            jittered_next_run_ms("*/5 * * * *", from, "task-a", &cfg, chrono_tz::UTC).unwrap();
        let jitter = jittered - base;
        let gap = 5 * 60 * 1000;

        assert!(jitter <= (cfg.recurring_frac * gap as f64) as u64);
        assert!(jitter <= cfg.recurring_cap_ms);
    }

    #[test]
    fn one_shot_no_jitter_on_off_minute_marks() {
        let from = utc_ms(2026, 1, 1, 9, 0);
        let cfg = JitterConfig::default();
        let jittered =
            one_shot_jittered_next_run_ms("17 10 * * *", from, "task-a", &cfg, chrono_tz::UTC)
                .unwrap();
        let base = next_run_ms("17 10 * * *", from, chrono_tz::UTC).unwrap();

        assert_eq!(jittered, base);
    }

    #[test]
    fn one_shot_lead_on_round_marks_clamped_to_from_ms() {
        let from = utc_ms(2026, 1, 1, 8, 59);
        let base = next_run_ms("0 9 * * *", from, chrono_tz::UTC).unwrap();
        let cfg = JitterConfig {
            one_shot_floor_ms: 90_000,
            ..JitterConfig::default()
        };
        let jittered =
            one_shot_jittered_next_run_ms("0 9 * * *", from, "task-a", &cfg, chrono_tz::UTC)
                .unwrap();

        assert!(jittered < base);
        assert_eq!(jittered, from);
    }

    #[test]
    fn tz_local_minute_check() {
        let from = utc_ms(2026, 1, 1, 3, 0);
        let tz = chrono_tz::Asia::Kolkata;
        let base = next_run_ms("0 9 * * *", from, tz).unwrap();
        let cfg = JitterConfig {
            one_shot_floor_ms: 90_000,
            one_shot_minute_mod: 60,
            ..JitterConfig::default()
        };
        let jittered =
            one_shot_jittered_next_run_ms("0 9 * * *", from, "task-a", &cfg, tz).unwrap();

        assert_eq!(base, utc_ms(2026, 1, 1, 3, 30));
        assert_eq!(jittered, base - 90_000);
    }
}
