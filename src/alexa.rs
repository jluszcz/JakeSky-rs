use crate::weather::Weather;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Timelike};
use chrono_tz::Tz;
use log::info;
use serde_json::{json, Value};

pub fn forecast(weather: Vec<Weather>) -> Result<Value> {
    let first_weather = weather
        .first()
        .ok_or_else(|| anyhow!("Cannot forecast empty weather"))?;

    for w in weather.iter().skip(1) {
        if w != first_weather {
            return forecast_different_weather(weather);
        }
    }

    forecast_same_weather(first_weather)
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

fn forecast_same_weather(weather: &Weather) -> Result<Value> {
    let weather = speakable_weather(weather);
    to_response(format!("All day, it will be {weather}."))
}

fn forecast_different_weather(weather: Vec<Weather>) -> Result<Value> {
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

    to_response(forecast.join(" "))
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
}
