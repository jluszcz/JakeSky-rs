use crate::weather::Weather;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Timelike};
use chrono_tz::Tz;
use log::info;
use serde_json::{json, Value};

pub fn forecast(weather: Vec<Weather>) -> Result<Value> {
    to_response(inner_forecast(weather)?)
}

fn inner_forecast(weather: Vec<Weather>) -> Result<String> {
    if weather.is_empty() {
        return Err(anyhow!("Cannot forecast empty weather"));
    }

    Ok(if is_all_same_weather(&weather) {
        forecast_same_weather(weather)
    } else {
        forecast_different_weather(weather)
    })
}

fn is_all_same_weather(weather: &[Weather]) -> bool {
    let first_weather = weather.first().unwrap();

    if weather.len() > 1 {
        for w in weather.iter().skip(1) {
            if w != first_weather {
                return false;
            }
        }
        return true;
    }

    false
}

fn to_response(forecast: String) -> Result<Value> {
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

fn forecast_same_weather(weather: Vec<Weather>) -> String {
    let weather = speakable_weather(weather.first().unwrap());
    format!("All day, it will be {weather}.")
}

fn forecast_different_weather(weather: Vec<Weather>) -> String {
    let mut forecast = Vec::with_capacity(weather.len());

    forecast.push(format!(
        "It's currently {}.",
        speakable_weather(&weather[0])
    ));

    if weather.len() > 1 {
        // Iterate over weather[1:-1]
        for w in weather.iter().skip(1).take(weather.len() - 2) {
            forecast.push(format!(
                "At {}, it will be {}.",
                speakable_timestamp(&w.timestamp),
                speakable_weather(w)
            ));
        }

        let last_weather = weather
            .last()
            .expect("A 'last' weather is guaranteed to exist");

        forecast.push(format!(
            "And at {} it will be {}.",
            speakable_timestamp(&last_weather.timestamp),
            speakable_weather(last_weather),
        ));
    }

    forecast.join(" ")
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
    inner_speakable_weather(weather.temp(), &weather.summary)
}

fn inner_speakable_weather(temp: f64, summary: &str) -> String {
    format!(
        "{:.0}{} and {}",
        temp.abs(),
        if temp < 0.0 { " below" } else { "" },
        summary
    )
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_speakable_weather() {
        assert!(inner_speakable_weather(72.0, "foo").starts_with("72 and"));
        assert!(inner_speakable_weather(-72.0, "foo").starts_with("72 below and"));
    }

    #[test]
    fn test_forecast_no_weather() {
        let weather: Vec<Weather> = Vec::new();

        assert!(inner_forecast(weather).is_err());
    }

    #[test]
    fn test_forecast_same_weather() -> Result<()> {
        let weather = vec![
            Weather::test::<String>(None),
            Weather::test::<String>(None),
            Weather::test::<String>(None),
        ];

        let forecast = inner_forecast(weather)?;

        assert!(forecast.starts_with("All day"));

        Ok(())
    }

    #[test]
    fn test_forecast_different_weather() -> Result<()> {
        let weather = vec![
            Weather::test(Some("sunny")),
            Weather::test(Some("rainy")),
            Weather::test(Some("cloudy")),
        ];

        let forecast = inner_forecast(weather)?;

        assert!(forecast.starts_with("It's currently"));

        Ok(())
    }
}
