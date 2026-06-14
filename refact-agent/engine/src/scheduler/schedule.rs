use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use super::cron_expr;
use super::types::{Job, Trigger};

pub trait ScheduleTarget {
    fn schedule_next_run_ms(&self, from_ms: u64, default_tz: Tz) -> Option<u64>;
}

impl ScheduleTarget for Job {
    fn schedule_next_run_ms(&self, from_ms: u64, default_tz: Tz) -> Option<u64> {
        match &self.trigger {
            Trigger::Cron { expr, tz } => {
                cron_next_run_ms(self, expr, tz.as_deref(), from_ms, default_tz)
            }
            Trigger::Interval { every_ms } => interval_next_run_ms(self, from_ms, *every_ms),
            Trigger::Once { at_ms } => once_next_run_ms(self, *at_ms),
            Trigger::Manual | Trigger::Webhook { .. } | Trigger::OnProcessExit { .. } => None,
        }
    }
}

impl ScheduleTarget for str {
    fn schedule_next_run_ms(&self, from_ms: u64, default_tz: Tz) -> Option<u64> {
        cron_expr::next_run_ms(self, from_ms, default_tz)
    }
}

impl ScheduleTarget for String {
    fn schedule_next_run_ms(&self, from_ms: u64, default_tz: Tz) -> Option<u64> {
        self.as_str().schedule_next_run_ms(from_ms, default_tz)
    }
}

pub fn next_run_ms<T: ScheduleTarget + ?Sized>(
    target: &T,
    from_ms: u64,
    default_tz: Tz,
) -> Option<u64> {
    target.schedule_next_run_ms(from_ms, default_tz)
}

pub fn parse_schedule(
    cron: Option<&str>,
    every: Option<&str>,
    at: Option<&str>,
    tz: Option<&str>,
    now_ms: u64,
) -> Result<Trigger, String> {
    let set_count = [cron.is_some(), every.is_some(), at.is_some()]
        .into_iter()
        .filter(|set| *set)
        .count();
    if set_count != 1 {
        return Err("exactly one of cron, every, or at must be set".to_string());
    }

    if let Some(expr) = cron {
        let expr = expr.trim();
        cron_expr::parse_cron(expr)?;
        let tz = match tz {
            Some(value) => {
                let value = value.trim();
                value
                    .parse::<Tz>()
                    .map_err(|_| format!("invalid timezone: {value}"))?;
                Some(value.to_string())
            }
            None => None,
        };
        return Ok(Trigger::Cron {
            expr: expr.to_string(),
            tz,
        });
    }

    if let Some(every) = every {
        return Ok(Trigger::Interval {
            every_ms: parse_duration_ms(every)?,
        });
    }

    let Some(at) = at else {
        return Err("exactly one of cron, every, or at must be set".to_string());
    };
    let at_ms = parse_at_ms(at, now_ms)?;
    Ok(Trigger::Once { at_ms })
}

fn parse_at_ms(value: &str, now_ms: u64) -> Result<u64, String> {
    let value = value.trim();
    let relative = value.strip_prefix("in ").unwrap_or(value).trim();
    if let Ok(duration_ms) = parse_duration_ms(relative) {
        return now_ms
            .checked_add(duration_ms)
            .ok_or_else(|| "relative schedule time overflowed".to_string());
    }
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| format!("invalid absolute schedule time: {error}"))?;
    let at_ms = u64::try_from(parsed.with_timezone(&Utc).timestamp_millis())
        .map_err(|_| "absolute schedule time is before the unix epoch".to_string())?;
    if at_ms <= now_ms {
        return Err("absolute schedule time must be in the future".to_string());
    }
    Ok(at_ms)
}

fn parse_duration_ms(value: &str) -> Result<u64, String> {
    let value = value.trim();
    let digit_len = value
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .last()
        .unwrap_or(0);
    if digit_len == 0 || digit_len == value.len() {
        return Err("duration must be a positive number followed by s, m, h, or d".to_string());
    }
    let amount = value[..digit_len]
        .parse::<u64>()
        .map_err(|_| "duration amount is invalid".to_string())?;
    if amount == 0 {
        return Err("duration must be greater than zero".to_string());
    }
    let unit_ms = match value[digit_len..].trim() {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 60 * 60_000,
        "d" => 24 * 60 * 60_000,
        _ => return Err("duration unit must be one of s, m, h, or d".to_string()),
    };
    amount
        .checked_mul(unit_ms)
        .ok_or_else(|| "duration overflowed".to_string())
}

fn cron_next_run_ms(
    job: &Job,
    expr: &str,
    tz: Option<&str>,
    from_ms: u64,
    default_tz: Tz,
) -> Option<u64> {
    if !job.recurring && (job.last_fired_at_ms.is_some() || job.fire_count > 0) {
        return None;
    }
    let tz = tz
        .and_then(|tz| tz.parse::<Tz>().ok())
        .unwrap_or(default_tz);
    cron_expr::next_run_ms(expr, from_ms, tz)
}

fn interval_next_run_ms(job: &Job, from_ms: u64, every_ms: u64) -> Option<u64> {
    if every_ms == 0 {
        return None;
    }
    let base_ms = job.last_fired_at_ms.unwrap_or(job.created_at_ms);
    let first = base_ms.checked_add(every_ms)?;
    if first > from_ms {
        return Some(first);
    }
    let ticks = from_ms.saturating_sub(base_ms) / every_ms + 1;
    base_ms.checked_add(ticks.checked_mul(every_ms)?)
}

fn once_next_run_ms(job: &Job, at_ms: u64) -> Option<u64> {
    if job.last_fired_at_ms.is_none() && job.fire_count == 0 {
        Some(at_ms)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::scheduler::types::{Action, AgentTarget, Delivery};

    fn utc_ms(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> u64 {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    }

    fn job(trigger: Trigger) -> Job {
        Job {
            id: "job-1".to_string(),
            description: "job".to_string(),
            enabled: true,
            durable: false,
            created_at_ms: 1_000,
            recurring: true,
            trigger,
            action: Action::AgentTurn {
                prompt: "prompt".to_string(),
                target: AgentTarget::ExistingChat {
                    chat_id: "chat-1".to_string(),
                },
                mode: None,
                model: None,
                tools: None,
            },
            delivery: Delivery::Chat,
            last_fired_at_ms: None,
            fire_count: 0,
            last_status: None,
            last_error: None,
            recent_runs: Vec::new(),
            paused_at_ms: None,
            trigger_at_ms: None,
            auto_expire_after_ms: 0,
        }
    }

    #[test]
    fn parse_schedule_accepts_valid_cron() {
        assert_eq!(
            parse_schedule(Some("*/5 * * * *"), None, None, None, 1_000).unwrap(),
            Trigger::Cron {
                expr: "*/5 * * * *".to_string(),
                tz: None,
            }
        );
    }

    #[test]
    fn parse_schedule_accepts_valid_intervals() {
        for (raw, expected) in [
            ("90s", 90_000),
            ("30m", 30 * 60_000),
            ("2h", 2 * 60 * 60_000),
            ("1d", 24 * 60 * 60_000),
        ] {
            assert_eq!(
                parse_schedule(None, Some(raw), None, None, 1_000).unwrap(),
                Trigger::Interval { every_ms: expected }
            );
        }
    }

    #[test]
    fn parse_schedule_accepts_absolute_at() {
        let now = utc_ms(2026, 1, 1, 0, 0);
        let at_ms = utc_ms(2026, 1, 1, 0, 5);

        assert_eq!(
            parse_schedule(None, None, Some("2026-01-01T00:05:00Z"), None, now).unwrap(),
            Trigger::Once { at_ms }
        );
    }

    #[test]
    fn parse_schedule_accepts_relative_at() {
        let now = 10_000;

        assert_eq!(
            parse_schedule(None, None, Some("in 30m"), None, now).unwrap(),
            Trigger::Once {
                at_ms: now + 30 * 60_000
            }
        );
        assert_eq!(
            parse_schedule(None, None, Some("2h"), None, now).unwrap(),
            Trigger::Once {
                at_ms: now + 2 * 60 * 60_000
            }
        );
    }

    #[test]
    fn parse_schedule_accepts_cron_timezone() {
        assert_eq!(
            parse_schedule(Some("0 9 * * *"), None, None, Some("Asia/Kolkata"), 1_000,).unwrap(),
            Trigger::Cron {
                expr: "0 9 * * *".to_string(),
                tz: Some("Asia/Kolkata".to_string()),
            }
        );
    }

    #[test]
    fn parse_schedule_ignores_timezone_for_non_cron() {
        assert_eq!(
            parse_schedule(None, Some("30m"), None, Some("Asia/Kolkata"), 1_000).unwrap(),
            Trigger::Interval {
                every_ms: 30 * 60_000
            }
        );
    }

    #[test]
    fn parse_schedule_requires_exactly_one_kind() {
        assert!(parse_schedule(None, None, None, None, 1_000).is_err());
        assert!(parse_schedule(Some("*/5 * * * *"), Some("30m"), None, None, 1_000).is_err());
    }

    #[test]
    fn parse_schedule_rejects_past_absolute_at() {
        let now = utc_ms(2026, 1, 1, 0, 5);

        assert!(parse_schedule(None, None, Some("2026-01-01T00:00:00Z"), None, now).is_err());
    }

    #[test]
    fn parse_schedule_rejects_bad_every() {
        for raw in ["0m", "nope", "30ms", ""] {
            assert!(parse_schedule(None, Some(raw), None, None, 1_000).is_err());
        }
    }

    #[test]
    fn parse_schedule_rejects_bad_timezone() {
        assert!(parse_schedule(Some("*/5 * * * *"), None, None, Some("Mars/Base"), 1_000).is_err());
    }

    #[test]
    fn cron_next_run_matches_cron_expr() {
        let from = utc_ms(2026, 1, 1, 0, 0);
        let job = job(Trigger::Cron {
            expr: "*/5 * * * *".to_string(),
            tz: None,
        });

        assert_eq!(
            next_run_ms(&job, from, chrono_tz::UTC),
            cron_expr::next_run_ms("*/5 * * * *", from, chrono_tz::UTC)
        );
    }

    #[test]
    fn cron_uses_job_timezone() {
        let from = utc_ms(2026, 1, 1, 0, 0);
        let job = job(Trigger::Cron {
            expr: "0 9 * * *".to_string(),
            tz: Some("Asia/Kolkata".to_string()),
        });

        assert_eq!(
            next_run_ms(&job, from, chrono_tz::UTC),
            cron_expr::next_run_ms("0 9 * * *", from, chrono_tz::Asia::Kolkata)
        );
    }

    #[test]
    fn cron_one_shot_none_after_fired() {
        let mut job = job(Trigger::Cron {
            expr: "*/5 * * * *".to_string(),
            tz: None,
        });
        job.recurring = false;
        job.last_fired_at_ms = Some(1_000);

        assert_eq!(next_run_ms(&job, 1_000, chrono_tz::UTC), None);
    }

    #[test]
    fn interval_advances_to_next_future_tick() {
        let mut job = job(Trigger::Interval { every_ms: 10 });
        job.created_at_ms = 1_000;

        assert_eq!(next_run_ms(&job, 1_000, chrono_tz::UTC), Some(1_010));
        assert_eq!(next_run_ms(&job, 1_031, chrono_tz::UTC), Some(1_040));

        job.last_fired_at_ms = Some(1_050);
        assert_eq!(next_run_ms(&job, 1_051, chrono_tz::UTC), Some(1_060));
    }

    #[test]
    fn interval_zero_never_schedules() {
        let job = job(Trigger::Interval { every_ms: 0 });

        assert_eq!(next_run_ms(&job, 1_000, chrono_tz::UTC), None);
    }

    #[test]
    fn once_returns_at_ms_until_fired() {
        let mut job = job(Trigger::Once { at_ms: 5_000 });

        assert_eq!(next_run_ms(&job, 1_000, chrono_tz::UTC), Some(5_000));
        assert_eq!(next_run_ms(&job, 6_000, chrono_tz::UTC), Some(5_000));

        job.fire_count = 1;
        assert_eq!(next_run_ms(&job, 6_000, chrono_tz::UTC), None);
    }

    #[test]
    fn manual_webhook_and_process_exit_never_schedule() {
        for trigger in [
            Trigger::Manual,
            Trigger::Webhook {
                hook_id: "hook-1".to_string(),
            },
            Trigger::OnProcessExit {
                match_kind: "any".to_string(),
            },
        ] {
            assert_eq!(next_run_ms(&job(trigger), 1_000, chrono_tz::UTC), None);
        }
    }
}
