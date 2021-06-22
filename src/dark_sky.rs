use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use log::{debug, info, trace};
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::{
    convert::TryFrom,
    env,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
};

#[derive(Debug)]
pub struct Weather {
    pub timestamp: DateTime<Tz>,
    pub summary: String,
    pub temperature: f64,
}

impl TryFrom<(DateTime<Tz>, &Value)> for Weather {
    type Error = anyhow::Error;

    fn try_from(value: (DateTime<Tz>, &Value)) -> Result<Self, Self::Error> {
        let (timestamp, dark_sky_data) = value;

        let summary = dark_sky_data["summary"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing weather summary"))?
            .to_owned();

        let temperature = dark_sky_data["temperature"]
            .as_f64()
            .ok_or_else(|| anyhow!("Missing temperature"))?;

        Ok(Weather {
            timestamp,
            summary,
            temperature,
        })
    }
}

pub async fn get_weather_info(
    use_cache: bool,
    dark_sky_api_key: String,
    latitude: f64,
    longitude: f64,
) -> Result<Vec<Weather>> {
    let cache_path = cache_path(latitude, longitude);

    let dark_sky_response = if let Some(cached) = try_cached(use_cache, &cache_path).await? {
        cached
    } else {
        let response = query(dark_sky_api_key, latitude, longitude).await?;
        try_write_cache(use_cache, &cache_path, &response).await?;
        response
    };

    Ok(parse_weather(dark_sky_response)?)
}

async fn query(dark_sky_api_key: String, latitude: f64, longitude: f64) -> Result<String> {
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
        .text()
        .await?;

    trace!("{}", response);

    Ok(response)
}

fn parse_weather(response: String) -> Result<Vec<Weather>> {
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

    let hours_of_interest = hours_of_interest(now, None, false);

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

fn hours_of_interest(
    current_time: DateTime<Tz>,
    hours: Option<Vec<u32>>,
    add_weekend_hour: bool,
) -> Vec<u32> {
    let mut hours = hours.unwrap_or_else(|| vec![8, 12, 18]);

    if add_weekend_hour && matches!(current_time.weekday(), Weekday::Sat | Weekday::Sun) {
        hours.push(22);
    }

    hours.sort_unstable();

    for n in 0..hours.len() {
        if current_time.hour() + 1 < hours[n] {
            hours = hours.split_off(n);
            break;
        }
    }

    debug!("Hours of Interest: {:?}", hours);

    hours
}

fn cache_path(latitude: f64, longitude: f64) -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!(
        "darksky-{}-{:.1}-{:.1}.json",
        Utc::today().format("%Y%m%d"),
        latitude.abs(),
        longitude.abs()
    ));

    path
}

async fn try_cached(use_cache: bool, cache_path: &Path) -> Result<Option<String>> {
    if use_cache && cache_path.exists() {
        debug!("Reading cache file: {:?}", cache_path);
        Ok(Some(fs::read_to_string(cache_path).await?))
    } else {
        Ok(None)
    }
}

async fn try_write_cache(use_cache: bool, cache_path: &Path, response: &str) -> Result<()> {
    if use_cache {
        debug!("Writing response to cache file: {:?}", cache_path);

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(cache_path)
            .await?;

        file.write_all(response.as_bytes()).await?;
    }
    Ok(())
}
