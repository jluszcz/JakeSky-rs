use crate::weather::{self, Weather};
use anyhow::{anyhow, Result};
use chrono::{DateTime, TimeZone, Timelike};
use chrono_tz::Tz;
use log::{debug, info, trace};
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::convert::TryFrom;

impl TryFrom<(DateTime<Tz>, &Value)> for Weather {
    type Error = anyhow::Error;

    fn try_from(value: (DateTime<Tz>, &Value)) -> Result<Self, Self::Error> {
        let (timestamp, dark_sky_data) = value;

        let summary = dark_sky_data["summary"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing weather summary"))?
            .to_owned();

        let summary = if "drizzle".eq_ignore_ascii_case(&summary) {
            "Drizzling".into()
        } else {
            summary
        };

        let temp = dark_sky_data["temperature"]
            .as_f64()
            .ok_or_else(|| anyhow!("Missing temperature"))?;

        let apparent_temp = dark_sky_data["apparentTemperature"].as_f64();

        Ok(Weather {
            timestamp,
            summary,
            temp,
            apparent_temp,
        })
    }
}

pub async fn query(dark_sky_api_key: String, latitude: f64, longitude: f64) -> Result<String> {
    // Since we only care about the current and hourly forecast for specific times, exclude some of the data in the response.
    let url = format!(
        "https://api.darksky.net/forecast/{}/{},{}?exclude=minutely,daily,flags",
        dark_sky_api_key, latitude, longitude
    );

    let mut headers = HeaderMap::with_capacity(2);
    headers.insert("Accept", "application/json".parse()?);
    headers.insert("Accept-Encoding", "gzip".parse()?);

    let client = reqwest::Client::new();

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    trace!("{}", response);

    Ok(response)
}

pub fn parse_weather(response: String) -> Result<Vec<Weather>> {
    let response: Value = serde_json::from_str(&response)?;
    let timezone = response["timezone"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing timezone"))?
        .parse::<Tz>()
        .map_err(|_| anyhow!("Failed to parse timezone"))?;

    debug!("Parsed timezone: {:?}", timezone);

    let now = timezone.timestamp(
        response["currently"]["time"]
            .as_i64()
            .ok_or_else(|| anyhow!("Missing current time"))?,
        0,
    );

    let hours_of_interest = weather::hours_of_interest(now, None, false);

    let mut weather = vec![Weather::try_from((now, &response["currently"]))?];

    let hourly_data = response["hourly"]["data"]
        .as_array()
        .ok_or_else(|| anyhow!("Missing hourly data"))?;

    for hourly_weather in hourly_data.iter() {
        let hourly_weather_time = timezone.timestamp(
            hourly_weather["time"]
                .as_i64()
                .ok_or_else(|| anyhow!("Missing time"))?,
            0,
        );

        if hourly_weather_time.date() > now.date() {
            debug!("{:?} is no longer relevant", hourly_weather_time);
            break;
        }

        if hourly_weather_time.hour() == now.hour() {
            debug!("Skipping current hour: {:?}", hourly_weather_time);
            continue;
        }

        if hours_of_interest.contains(&hourly_weather_time.hour()) {
            weather.push(Weather::try_from((hourly_weather_time, hourly_weather))?);
        }
    }

    for w in weather.iter() {
        info!("{:?}", w);
    }

    Ok(weather)
}
