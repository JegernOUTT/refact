use std::str::FromStr;

use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;

#[derive(Clone, Debug)]
pub struct CronSchedule {
    schedule: Schedule,
}

impl CronSchedule {
    pub fn upcoming(&self, tz: Tz) -> impl Iterator<Item = chrono::DateTime<Tz>> + '_ {
        self.schedule.upcoming(tz)
    }
}

pub fn parse_cron(expr: &str) -> Result<CronSchedule, String> {
    let normalized = normalize_five_field_cron(expr)?;
    let schedule = Schedule::from_str(&normalized)
        .map_err(|error| format!("Invalid cron expression: {error}"))?;
    Ok(CronSchedule { schedule })
}

pub fn next_run_ms(expr: &str, from_ms: u64, tz: Tz) -> Option<u64> {
    let schedule = parse_cron(expr).ok()?;
    let from = Utc
        .timestamp_millis_opt(from_ms as i64)
        .single()?
        .with_timezone(&tz);
    schedule
        .schedule
        .after(&from)
        .next()
        .and_then(|datetime| u64::try_from(datetime.timestamp_millis()).ok())
}

pub fn human_schedule(expr: &str) -> String {
    let fields = match five_fields(expr) {
        Some(fields) => fields,
        None => return expr.to_string(),
    };

    match fields.as_slice() {
        ["*/5", "*", "*", "*", "*"] => "every 5 minutes".to_string(),
        ["*/15", "*", "*", "*", "*"] => "every 15 minutes".to_string(),
        ["0", "9", "*", "*", "1-5"] => "weekdays at 9am".to_string(),
        ["0", "9", "*", "*", "MON-FRI"] => "weekdays at 9am".to_string(),
        ["0", "0", "*", "*", "*"] => "daily at midnight".to_string(),
        [minute, "*", "*", "*", "*"] if minute.parse::<u8>().is_ok() => {
            format!("hourly at :{minute}")
        }
        ["0", hour, "*", "*", "*"] => daily_at(hour, expr),
        _ => expr.to_string(),
    }
}

fn normalize_five_field_cron(expr: &str) -> Result<String, String> {
    let fields = five_fields(expr)
        .ok_or_else(|| "Cron expression must have exactly 5 fields".to_string())?;
    Ok(format!("0 {}", fields.join(" ")))
}

fn five_fields(expr: &str) -> Option<Vec<&str>> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() == 5 {
        Some(fields)
    } else {
        None
    }
}

fn daily_at(hour: &str, raw: &str) -> String {
    let Ok(hour) = hour.parse::<u8>() else {
        return raw.to_string();
    };
    if hour > 23 {
        return raw.to_string();
    }
    match hour {
        0 => "daily at midnight".to_string(),
        1..=11 => format!("daily at {hour}am"),
        12 => "daily at noon".to_string(),
        _ => format!("daily at {}pm", hour - 12),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_every_5_minutes() {
        assert!(parse_cron("*/5 * * * *").is_ok());
    }

    #[test]
    fn parse_weekdays_at_9am() {
        assert!(parse_cron("0 9 * * 1-5").is_ok());
    }

    #[test]
    fn next_run_strictly_after_from_ms() {
        let from = Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 5, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let next = next_run_ms("*/5 * * * *", from, chrono_tz::UTC).unwrap();
        let expected = Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 10, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        assert_eq!(next, expected);
    }

    #[test]
    fn human_schedule_well_known_cases() {
        assert_eq!(human_schedule("*/5 * * * *"), "every 5 minutes");
        assert_eq!(human_schedule("0 9 * * 1-5"), "weekdays at 9am");
        assert_eq!(human_schedule("0 0 * * *"), "daily at midnight");
        assert_eq!(human_schedule("17 * * * *"), "hourly at :17");
        assert_eq!(human_schedule("1 2 3 4 5"), "1 2 3 4 5");
    }
}
