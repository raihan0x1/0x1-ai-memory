//! Time anchoring shared by every Briefing, Recall and MCP search result, so agents can tell
//! "14 minutes ago" from "yesterday" instead of guessing.

use chrono::{DateTime, TimeZone};
use std::fmt::Display;

/// When the Briefing, Recall or search result was generated: local time with UTC offset.
pub fn generated_at<Tz: TimeZone>(now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    now.format("%Y-%m-%d %H:%M UTC%:z").to_string()
}

/// The line that opens injected memory context.
pub fn generated_at_line<Tz: TimeZone>(now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    format!(
        "Generated at {}. Memory times are local; each age counts back from this moment.",
        generated_at(now)
    )
}

/// `"19:06, 14m before this briefing"`: the memory's local time (with date unless same day as
/// `now`) and its age relative to `now`, never a bare "ago".
pub fn memory_when<Tz: TimeZone>(updated_at: &str, now: &DateTime<Tz>, reference: &str) -> String
where
    Tz::Offset: Display,
{
    let Ok(ts) = DateTime::parse_from_rfc3339(updated_at) else {
        return updated_at.to_string();
    };
    let ts = ts.with_timezone(&now.timezone());
    let clock = if ts.date_naive() == now.date_naive() {
        ts.format("%H:%M")
    } else {
        ts.format("%Y-%m-%d %H:%M")
    };
    let mins = (now.clone() - ts).num_minutes().max(0);
    let age = match mins {
        0 => "<1m".to_string(),
        1..60 => format!("{mins}m"),
        60..1440 => format!("{}h {}m", mins / 60, mins % 60),
        _ => format!("{}d {}h", mins / 1440, mins % 1440 / 60),
    };
    format!("{clock}, {age} before {reference}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn ages_are_relative_to_generation_time_in_local_offset() {
        let tz = FixedOffset::east_opt(7 * 3600).unwrap();
        let now = tz.with_ymd_and_hms(2026, 11, 27, 19, 20, 0).unwrap();
        assert_eq!(generated_at(&now), "2026-11-27 19:20 UTC+07:00");
        let at = |s: &str| memory_when(s, &now, "this briefing");
        assert_eq!(
            at("2026-11-27T12:06:00Z"),
            "19:06, 14m before this briefing"
        );
        assert_eq!(
            at("2026-11-27T12:19:30Z"),
            "19:19, <1m before this briefing"
        );
        assert_eq!(
            at("2026-11-27T09:05:00Z"),
            "16:05, 3h 15m before this briefing"
        );
        // Yesterday in local time: the date is shown.
        assert_eq!(
            at("2026-11-26T16:50:00Z"),
            "2026-11-26 23:50, 19h 30m before this briefing"
        );
        assert_eq!(
            at("2026-11-25T12:20:00Z"),
            "2026-11-25 19:20, 2d 0h before this briefing"
        );
        // Clock skew never yields a negative age; unparseable input passes through.
        assert_eq!(
            at("2026-11-27T12:25:00Z"),
            "19:25, <1m before this briefing"
        );
        assert_eq!(at("not-a-time"), "not-a-time");
    }
}
