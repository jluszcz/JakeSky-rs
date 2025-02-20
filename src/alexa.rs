use crate::weather::Weather;
use anyhow::{Result, anyhow};
use chrono::{DateTime, Timelike};
use chrono_tz::Tz;
use log::info;
use serde_json::{Value, json};

pub fn forecast(weather: Vec<Weather>) -> Result<Value> {
    let forecast = to_forecast(weather)?.join(" ");

    info!(r#"Forecast: "{}""#, forecast);

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

fn to_forecast(weather: Vec<Weather>) -> Result<Vec<String>> {
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

    if weather.len() > 1 {
        if let Some(w) = weather.last() {
            forecast.push(format!(
                "{} {} it will be {}.",
                if weather.len() > 2 { "And at" } else { "At" },
                speakable_timestamp(&w.timestamp),
                speakable_weather(w),
            ));
        }
    }

    Ok(forecast)
}

fn speakable_timestamp(timestamp: &DateTime<Tz>) -> String {
    match timestamp.hour() {
        0 => "midnight".into(),
        12 => "noon".into(),
        _ => {
            let (pm, hour) = timestamp.hour12();
            format!("{} {}", hour, if pm { "PM" } else { "AM" })
        }
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
        assert!(to_forecast(Vec::new()).is_err());
    }

    #[test]
    fn test_to_forecast_one_weather() -> Result<()> {
        let weather = vec![Weather::test(Some("1"))];
        let forecast = to_forecast(weather)?;

        assert_eq!(1, forecast.len());
        assert!(!forecast[0].contains("And"));

        Ok(())
    }

    #[test]
    fn test_to_forecast_two_weather() -> Result<()> {
        let weather = vec![Weather::test(Some("1")), Weather::test(Some("2"))];
        let forecast = to_forecast(weather)?;

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
        let forecast = to_forecast(weather)?;

        assert_eq!(3, forecast.len());
        assert!(!forecast[1].contains("And"));
        assert!(forecast[2].contains("And"));

        Ok(())
    }
}
