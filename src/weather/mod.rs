use anyhow::Result;
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use log::debug;
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

pub mod dark_sky;
pub mod open_weather;

#[derive(Debug)]
pub struct Weather {
    pub timestamp: DateTime<Tz>,
    pub summary: String,
    pub temp: f64,
    pub apparent_temp: Option<f64>,
}

#[derive(Debug)]
pub enum WeatherProvider {
    DarkSky,
    OpenWeather,
}

impl WeatherProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::DarkSky => "darksky",
            Self::OpenWeather => "openweather",
        }
    }

    async fn query(&self, api_key: String, latitude: f64, longitude: f64) -> Result<String> {
        match self {
            Self::DarkSky => dark_sky::query(api_key, latitude, longitude).await,
            Self::OpenWeather => open_weather::query(api_key, latitude, longitude).await,
        }
    }

    fn parse_weather(&self, response: String) -> Result<Vec<Weather>> {
        match self {
            Self::DarkSky => dark_sky::parse_weather(response),
            Self::OpenWeather => open_weather::parse_weather(response),
        }
    }
}

pub async fn get_weather_info(
    weather_provider: &WeatherProvider,
    use_cache: bool,
    api_key: String,
    latitude: f64,
    longitude: f64,
) -> Result<Vec<Weather>> {
    let cache_path = cache_path(weather_provider.id(), latitude, longitude);

    let response = if let Some(cached) = try_cached(use_cache, &cache_path).await? {
        cached
    } else {
        let response = weather_provider.query(api_key, latitude, longitude).await?;
        try_write_cache(use_cache, &cache_path, &response).await?;
        response
    };

    Ok(weather_provider.parse_weather(response)?)
}

fn cache_path(weather_provider_id: &'static str, latitude: f64, longitude: f64) -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!(
        "{}-{}-{:.1}-{:.1}.json",
        weather_provider_id,
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

pub fn hours_of_interest(
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
