use crate::weather::{Weather, WeatherAlert};
use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use log::info;
use serde_json::{Value, json};

pub fn forecast(weather: Vec<Weather>, alerts: Vec<WeatherAlert>) -> Result<Value> {
    let forecast = to_forecast(weather, alerts)?.join(" ");

    info!(r#"Forecast: "{forecast}""#);

    Ok(json!({
        "version": "1.0",
        "response": {
            "outputSpeech": {
                "type": "PlainText",
                "text": forecast,
            }
        }
    }))
}

fn to_forecast(weather: Vec<Weather>, alerts: Vec<WeatherAlert>) -> Result<Vec<String>> {
    if weather.is_empty() {
        return Err(anyhow!("Weather cannot be empty"));
    }

    let mut forecast = Vec::with_capacity(weather.len());

    forecast.push(format!(
        "It's currently {}.",
        speakable_weather(weather.first().unwrap())
    ));

    for w in weather.iter().skip(1).take(weather.len().saturating_sub(2)) {
        forecast.push(format!(
            "At {}, it will be {}.",
            speakable_timestamp(&w.timestamp),
            speakable_weather(w)
        ));
    }

    if weather.len() > 1
        && let Some(w) = weather.last()
    {
        forecast.push(format!(
            "{} {} it will be {}.",
            if weather.len() > 2 { "And at" } else { "At" },
            speakable_timestamp(&w.timestamp),
            speakable_weather(w),
        ));
    }

    if !alerts.is_empty() {
        forecast.push(format_alerts(&alerts));
    }

    Ok(forecast)
}

fn speakable_timestamp(timestamp: &DateTime<Tz>) -> String {
    match timestamp.hour() {
        0 => "midnight".to_string(),
        12 => "noon".to_string(),
        _ => timestamp.format("%-I%P").to_string(),
    }
}

fn speakable_weather(weather: &Weather) -> String {
    let temp = weather.apparent_temp.unwrap_or(weather.temp) as i64;
    inner_speakable_weather(temp, &weather.summary)
}

fn inner_speakable_weather(temp: i64, summary: &str) -> String {
    format!(
        "{:.0}{} and {}",
        temp.abs(),
        if temp < 0 { " below" } else { "" },
        summary
    )
}

fn format_alerts(alerts: &[WeatherAlert]) -> String {
    let count = alerts.len();
    let mut parts = Vec::new();

    // Announce first 2 alerts with event name and time range
    for (index, alert) in alerts.iter().take(2).enumerate() {
        let time_range = format_alert_timerange(&alert.start, &alert.end);
        let event_lower = alert.event.to_lowercase();

        if index == 0 {
            parts.push(format!("There is a {} {}", event_lower, time_range));
        } else {
            parts.push(format!("And a {} {}", event_lower, time_range));
        }
    }

    if count > 2 {
        let remaining = count - 2;
        let plural = if remaining == 1 { "alert" } else { "alerts" };
        parts.push(format!("And {} more {}", remaining, plural));
    }

    parts.join(". ") + "."
}

fn format_alert_timerange(start: &DateTime<Tz>, end: &DateTime<Tz>) -> String {
    // Format as "from 7am tomorrow through 8pm Monday" or "from midnight through 11am today"
    let start_time = speakable_timestamp(start);
    let end_time = speakable_timestamp(end);

    let now = start.timezone().from_utc_datetime(&Utc::now().naive_utc());
    let start_day = relative_day(start, &now);
    let end_day = relative_day(end, &now);

    // Omit start time if alert started in the past
    if start < &now {
        format!("until {} {}", end_time, end_day)
    } else if start_day == end_day {
        format!("from {} through {} {}", start_time, end_time, end_day)
    } else {
        format!(
            "from {} {} through {} {}",
            start_time, start_day, end_time, end_day
        )
    }
}

fn relative_day(dt: &DateTime<Tz>, now: &DateTime<Tz>) -> String {
    let days_diff = dt
        .date_naive()
        .signed_duration_since(now.date_naive())
        .num_days();
    match days_diff {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        -1 => "yesterday".to_string(),
        _ => dt.format("%A").to_string(), // Day of week
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_speakable_weather() {
        assert!(inner_speakable_weather(72, "foo").starts_with("72 and"));
        assert!(inner_speakable_weather(-72, "foo").starts_with("72 below and"));
    }

    #[test]
    fn test_to_forecast_empty() {
        assert!(to_forecast(Vec::new(), Vec::new()).is_err());
    }

    #[test]
    fn test_to_forecast_one_weather() -> Result<()> {
        let weather = vec![Weather::test(Some("1"))];
        let forecast = to_forecast(weather, Vec::new())?;

        assert_eq!(1, forecast.len());
        assert!(!forecast[0].contains("And"));

        Ok(())
    }

    #[test]
    fn test_to_forecast_two_weather() -> Result<()> {
        let weather = vec![Weather::test(Some("1")), Weather::test(Some("2"))];
        let forecast = to_forecast(weather, Vec::new())?;

        assert_eq!(2, forecast.len());
        assert!(!forecast[1].contains("And"));

        Ok(())
    }

    #[test]
    fn test_to_forecast_multiple_weather() -> Result<()> {
        let weather = vec![
            Weather::test(Some("1")),
            Weather::test(Some("2")),
            Weather::test(Some("3")),
        ];
        let forecast = to_forecast(weather, Vec::new())?;

        assert_eq!(3, forecast.len());
        assert!(!forecast[1].contains("And"));
        assert!(forecast[2].contains("And"));

        Ok(())
    }

    #[test]
    fn test_to_forecast_with_one_alert() -> Result<()> {
        use chrono::Duration;

        let weather = vec![Weather::test(Some("sunny"))];
        let now = Utc::now().with_timezone(&Tz::UTC);

        let alerts = vec![WeatherAlert {
            event: "Small Craft Advisory".to_string(),
            sender_name: "NWS".to_string(),
            start: now + Duration::hours(2),
            end: now + Duration::hours(18),
            description: "Test alert".to_string(),
        }];

        let forecast = to_forecast(weather, alerts)?;

        assert_eq!(2, forecast.len());
        assert!(forecast[1].contains("There is a"));
        assert!(forecast[1].contains("small craft advisory"));

        Ok(())
    }

    #[test]
    fn test_to_forecast_with_multiple_alerts() -> Result<()> {
        use chrono::Duration;

        let weather = vec![Weather::test(Some("sunny"))];
        let now = Utc::now().with_timezone(&Tz::UTC);

        let alerts = vec![
            WeatherAlert {
                event: "Winter Storm Warning".to_string(),
                sender_name: "NWS".to_string(),
                start: now + Duration::hours(6),
                end: now + Duration::hours(30),
                description: "Test alert 1".to_string(),
            },
            WeatherAlert {
                event: "Flood Watch".to_string(),
                sender_name: "NWS".to_string(),
                start: now + Duration::hours(8),
                end: now + Duration::hours(32),
                description: "Test alert 2".to_string(),
            },
            WeatherAlert {
                event: "High Wind Warning".to_string(),
                sender_name: "NWS".to_string(),
                start: now + Duration::hours(10),
                end: now + Duration::hours(34),
                description: "Test alert 3".to_string(),
            },
        ];

        let forecast = to_forecast(weather, alerts)?;

        assert_eq!(2, forecast.len());
        assert!(forecast[1].contains("There is a"));
        assert!(forecast[1].contains("winter storm warning"));
        assert!(forecast[1].contains("flood watch"));
        assert!(forecast[1].contains("And 1 more alert"));

        Ok(())
    }

    #[test]
    fn test_speakable_timestamp_midnight() {
        use chrono::NaiveDate;
        let dt = Tz::UTC
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2024, 1, 15)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(speakable_timestamp(&dt), "midnight");
    }

    #[test]
    fn test_speakable_timestamp_noon() {
        use chrono::NaiveDate;
        let dt = Tz::UTC
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2024, 1, 15)
                    .unwrap()
                    .and_hms_opt(12, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(speakable_timestamp(&dt), "noon");
    }

    #[test]
    fn test_speakable_timestamp_normal_hours() {
        use chrono::NaiveDate;

        let am_time = Tz::UTC
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2024, 1, 15)
                    .unwrap()
                    .and_hms_opt(8, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(speakable_timestamp(&am_time), "8am");

        let pm_time = Tz::UTC
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2024, 1, 15)
                    .unwrap()
                    .and_hms_opt(18, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(speakable_timestamp(&pm_time), "6pm");
    }

    #[test]
    fn test_format_alert_timerange_same_day() {
        use chrono::Duration;

        // Create alert in the future on the same day
        let now = Utc::now().with_timezone(&Tz::UTC);

        // Start 2 hours from now, end 8 hours from now (both in future, same day)
        let start = now + Duration::hours(2);
        let end = now + Duration::hours(8);

        let result = format_alert_timerange(&start, &end);
        // Should have "from" since start is in the future
        assert!(result.contains("from"));
        assert!(result.contains("through"));
        // Should only have one day mention at the end when same day
        assert!(
            !result.matches(" today ").any(|_| true),
            "Should not have 'today' in the middle"
        );
        assert!(result.ends_with("today") || result.ends_with("tomorrow"));
    }

    #[test]
    fn test_format_alert_timerange_different_days() {
        use chrono::Duration;

        // Use tomorrow to ensure start is always in the future
        let now = Utc::now().with_timezone(&Tz::UTC);
        let tomorrow = (now + Duration::days(1)).date_naive();

        let start = Tz::UTC
            .from_local_datetime(&tomorrow.and_hms_opt(8, 0, 0).unwrap())
            .unwrap();

        let end = start + Duration::days(2) + Duration::hours(4); // 12pm two days later

        let result = format_alert_timerange(&start, &end);
        assert!(result.contains("from 8am"));
        assert!(result.contains("through noon"));
        // Should have two day mentions (different days)
        assert!(result.contains("8am "));
        assert!(result.contains(" noon "));
        // Verify format has both day mentions
        let parts: Vec<&str> = result.split_whitespace().collect();
        assert!(parts.len() >= 6); // "from 8am [day] through noon [day]"
    }

    #[test]
    fn test_format_alert_timerange_yesterday_to_today() {
        use chrono::Duration;

        // Create an alert that started yesterday and ends today
        let now = Utc::now().with_timezone(&Tz::UTC);
        let today = now.date_naive();
        let yesterday = today - Duration::days(1);

        let start = Tz::UTC
            .from_local_datetime(&yesterday.and_hms_opt(23, 0, 0).unwrap()) // 11pm yesterday
            .unwrap();

        let end = Tz::UTC
            .from_local_datetime(&today.and_hms_opt(10, 0, 0).unwrap()) // 10am today
            .unwrap();

        let result = format_alert_timerange(&start, &end);
        // Should omit "yesterday" and "from", use "until" for past alerts
        assert!(
            !result.contains("yesterday"),
            "Result should not contain 'yesterday': {}",
            result
        );
        assert!(
            !result.contains("from"),
            "Result should not contain 'from': {}",
            result
        );
        assert!(
            result.starts_with("until"),
            "Result should start with 'until': {}",
            result
        );
        assert!(result.contains("10am"));
        assert!(result.contains("today"));
    }
}
