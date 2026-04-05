use crate::ai::AlertSummarize;
use crate::alert_summary::{extract_phenomenon, is_vague_event};
use crate::weather::{Weather, WeatherAlert};
use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use log::{info, warn};
use serde_json::{Value, json};

/// How to refer to an alert when reading it aloud.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AlertSubject {
    /// Use the event name as a noun (e.g. "winter storm warning"). Spoken with
    /// an article and "is": "There is a winter storm warning ...".
    Event(String),
    /// Use a concrete phenomenon extracted from the description (e.g.
    /// "areas of fog"). Spoken without an article and with "will be":
    /// "There will be areas of fog ...".
    Phenomenon(String),
}

async fn alert_subject<S: AlertSummarize>(
    alert: &WeatherAlert,
    summarizer: Option<&S>,
) -> AlertSubject {
    let event_lower = alert.event.to_lowercase();

    if !is_vague_event(&alert.event) {
        return AlertSubject::Event(event_lower);
    }

    if let Some(phenomenon) = extract_phenomenon(&alert.description) {
        return AlertSubject::Phenomenon(phenomenon);
    }

    if let Some(summarizer) = summarizer {
        match summarizer
            .summarize_alert(&alert.event, &alert.description)
            .await
        {
            Ok(phrase) => return AlertSubject::Phenomenon(phrase),
            Err(e) => warn!("Bedrock summarization failed for {:?}: {e}", alert.event),
        }
    }

    AlertSubject::Event(event_lower)
}

pub async fn forecast<S: AlertSummarize>(
    weather: Vec<Weather>,
    alerts: Vec<WeatherAlert>,
    summarizer: Option<&S>,
) -> Result<Value> {
    let forecast = to_forecast(weather, alerts, summarizer).await?.join(" ");

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

async fn to_forecast<S: AlertSummarize>(
    weather: Vec<Weather>,
    alerts: Vec<WeatherAlert>,
    summarizer: Option<&S>,
) -> Result<Vec<String>> {
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
        forecast.push(format_alerts(&alerts, summarizer).await);
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

async fn format_alerts<S: AlertSummarize>(
    alerts: &[WeatherAlert],
    summarizer: Option<&S>,
) -> String {
    let count = alerts.len();
    let mut parts = Vec::new();

    // Announce first 2 alerts, preferring a concrete phenomenon over the
    // generic event name when the event is vague (e.g. "Special Weather
    // Statement").
    for (index, alert) in alerts.iter().take(2).enumerate() {
        let time_range = format_alert_timerange(&alert.start, &alert.end);
        let subject = alert_subject(alert, summarizer).await;

        let phrase = match (index, &subject) {
            (0, AlertSubject::Event(name)) => format!("There is a {} {}", name, time_range),
            (0, AlertSubject::Phenomenon(p)) => format!("There will be {} {}", p, time_range),
            (_, AlertSubject::Event(name)) => format!("And a {} {}", name, time_range),
            (_, AlertSubject::Phenomenon(p)) => format!("And {} {}", p, time_range),
        };
        parts.push(phrase);
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

    /// Stub summarizer for exercising the LLM fallback path without hitting
    /// AWS. `phrase: Some(..)` produces a successful response; `None`
    /// produces an error so the caller's fallback-to-event-name can be
    /// tested.
    struct StubSummarizer {
        phrase: Option<String>,
    }

    impl AlertSummarize for StubSummarizer {
        async fn summarize_alert(&self, _event: &str, _description: &str) -> Result<String> {
            self.phrase
                .clone()
                .ok_or_else(|| anyhow!("stub summarizer failure"))
        }
    }

    /// Typed `None` for tests that don't need a summarizer, so type
    /// inference picks up the generic parameter.
    const NO_SUMMARIZER: Option<&StubSummarizer> = None;

    #[test]
    fn test_speakable_weather() {
        assert!(inner_speakable_weather(72, "foo").starts_with("72 and"));
        assert!(inner_speakable_weather(-72, "foo").starts_with("72 below and"));
    }

    #[tokio::test]
    async fn test_to_forecast_empty() {
        assert!(
            to_forecast(Vec::new(), Vec::new(), NO_SUMMARIZER)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_to_forecast_one_weather() -> Result<()> {
        let weather = vec![Weather::test(Some("1"))];
        let forecast = to_forecast(weather, Vec::new(), NO_SUMMARIZER).await?;

        assert_eq!(1, forecast.len());
        assert!(!forecast[0].contains("And"));

        Ok(())
    }

    #[tokio::test]
    async fn test_to_forecast_two_weather() -> Result<()> {
        let weather = vec![Weather::test(Some("1")), Weather::test(Some("2"))];
        let forecast = to_forecast(weather, Vec::new(), NO_SUMMARIZER).await?;

        assert_eq!(2, forecast.len());
        assert!(!forecast[1].contains("And"));

        Ok(())
    }

    #[tokio::test]
    async fn test_to_forecast_multiple_weather() -> Result<()> {
        let weather = vec![
            Weather::test(Some("1")),
            Weather::test(Some("2")),
            Weather::test(Some("3")),
        ];
        let forecast = to_forecast(weather, Vec::new(), NO_SUMMARIZER).await?;

        assert_eq!(3, forecast.len());
        assert!(!forecast[1].contains("And"));
        assert!(forecast[2].contains("And"));

        Ok(())
    }

    #[tokio::test]
    async fn test_to_forecast_with_one_alert() -> Result<()> {
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

        let forecast = to_forecast(weather, alerts, NO_SUMMARIZER).await?;

        assert_eq!(2, forecast.len());
        assert!(forecast[1].contains("There is a"));
        assert!(forecast[1].contains("small craft advisory"));

        Ok(())
    }

    #[tokio::test]
    async fn test_to_forecast_with_multiple_alerts() -> Result<()> {
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

        let forecast = to_forecast(weather, alerts, NO_SUMMARIZER).await?;

        assert_eq!(2, forecast.len());
        assert!(forecast[1].contains("There is a"));
        assert!(forecast[1].contains("winter storm warning"));
        assert!(forecast[1].contains("flood watch"));
        assert!(forecast[1].contains("And 1 more alert"));

        Ok(())
    }

    #[tokio::test]
    async fn test_to_forecast_vague_alert_with_phenomenon_uses_will_be() -> Result<()> {
        use chrono::Duration;

        let weather = vec![Weather::test(Some("sunny"))];
        let now = Utc::now().with_timezone(&Tz::UTC);

        let alerts = vec![WeatherAlert {
            event: "Special Weather Statement".to_string(),
            sender_name: "NWS".to_string(),
            start: now - Duration::hours(1),
            end: now + Duration::hours(2),
            description: "Areas of fog continue early this morning, with visibilities \
                          ranging between one and one-quarter mile."
                .to_string(),
        }];

        let forecast = to_forecast(weather, alerts, NO_SUMMARIZER).await?;

        assert_eq!(2, forecast.len());
        assert!(
            forecast[1].contains("There will be areas of fog"),
            "Expected phenomenon-based phrasing, got: {}",
            forecast[1]
        );
        assert!(!forecast[1].contains("special weather statement"));

        Ok(())
    }

    #[tokio::test]
    async fn test_to_forecast_vague_alert_without_phenomenon_falls_back() -> Result<()> {
        use chrono::Duration;

        let weather = vec![Weather::test(Some("sunny"))];
        let now = Utc::now().with_timezone(&Tz::UTC);

        let alerts = vec![WeatherAlert {
            event: "Special Weather Statement".to_string(),
            sender_name: "NWS".to_string(),
            start: now - Duration::hours(1),
            end: now + Duration::hours(2),
            description: "A generic advisory with no specific phenomenon mentioned.".to_string(),
        }];

        let forecast = to_forecast(weather, alerts, NO_SUMMARIZER).await?;

        assert_eq!(2, forecast.len());
        assert!(forecast[1].contains("There is a special weather statement"));

        Ok(())
    }

    #[tokio::test]
    async fn test_to_forecast_mixed_alerts() -> Result<()> {
        use chrono::Duration;

        let weather = vec![Weather::test(Some("sunny"))];
        let now = Utc::now().with_timezone(&Tz::UTC);

        let alerts = vec![
            WeatherAlert {
                event: "Special Weather Statement".to_string(),
                sender_name: "NWS".to_string(),
                start: now - Duration::hours(1),
                end: now + Duration::hours(2),
                description: "Areas of dense fog through late morning.".to_string(),
            },
            WeatherAlert {
                event: "Flood Watch".to_string(),
                sender_name: "NWS".to_string(),
                start: now + Duration::hours(3),
                end: now + Duration::hours(12),
                description: "Flooding possible in low areas.".to_string(),
            },
        ];

        let forecast = to_forecast(weather, alerts, NO_SUMMARIZER).await?;

        assert_eq!(2, forecast.len());
        assert!(forecast[1].contains("There will be dense fog"));
        assert!(forecast[1].contains("And a flood watch"));

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
        use chrono::NaiveDate;

        // Use fixed noon time so start and end always stay on the same day regardless of when
        // the test runs. noon + 6h = 6pm, both on the same date.
        let noon = Tz::UTC
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2099, 6, 15)
                    .unwrap()
                    .and_hms_opt(12, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        let start = noon;
        let end = noon + chrono::Duration::hours(6);

        let result = format_alert_timerange(&start, &end);
        // speakable_timestamp(noon) = "noon", speakable_timestamp(6pm) = "6pm"
        // Same day → day label should appear only once at the end
        assert!(
            result.starts_with("from noon through 6pm "),
            "Expected 'from noon through 6pm <day>', got: {}",
            result
        );
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

    #[tokio::test]
    async fn test_vague_alert_uses_stub_summarizer() -> Result<()> {
        use chrono::Duration;

        let weather = vec![Weather::test(Some("sunny"))];
        let now = Utc::now().with_timezone(&Tz::UTC);

        let alerts = vec![WeatherAlert {
            event: "Special Weather Statement".to_string(),
            sender_name: "NWS".to_string(),
            start: now - Duration::hours(1),
            end: now + Duration::hours(2),
            // Description that won't match any rule-based phenomenon
            description: "Unusual conditions in the area today.".to_string(),
        }];

        let stub = StubSummarizer {
            phrase: Some("gusty crosswinds".to_string()),
        };

        let forecast = to_forecast(weather, alerts, Some(&stub)).await?;

        assert_eq!(2, forecast.len());
        assert!(
            forecast[1].contains("There will be gusty crosswinds"),
            "Expected phenomenon from stub summarizer, got: {}",
            forecast[1]
        );
        assert!(!forecast[1].contains("special weather statement"));

        Ok(())
    }

    #[tokio::test]
    async fn test_vague_alert_falls_back_on_summarizer_error() -> Result<()> {
        use chrono::Duration;

        let weather = vec![Weather::test(Some("sunny"))];
        let now = Utc::now().with_timezone(&Tz::UTC);

        let alerts = vec![WeatherAlert {
            event: "Special Weather Statement".to_string(),
            sender_name: "NWS".to_string(),
            start: now - Duration::hours(1),
            end: now + Duration::hours(2),
            description: "Unusual conditions in the area today.".to_string(),
        }];

        let stub = StubSummarizer { phrase: None };

        let forecast = to_forecast(weather, alerts, Some(&stub)).await?;

        assert_eq!(2, forecast.len());
        assert!(
            forecast[1].contains("There is a special weather statement"),
            "Expected event-name fallback, got: {}",
            forecast[1]
        );

        Ok(())
    }
}
