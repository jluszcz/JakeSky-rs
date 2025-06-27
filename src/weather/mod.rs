use anyhow::{Result, anyhow};
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use log::{debug, trace};
use std::env;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

pub mod accu_weather;
pub mod open_weather;

#[derive(Debug)]
pub struct Weather {
    pub timestamp: DateTime<Tz>,
    pub summary: String,
    pub temp: f64,
    pub apparent_temp: Option<f64>,
}

impl Weather {
    #[cfg(test)]
    pub fn test<S>(summary: Option<S>) -> Self
    where
        S: Into<String>,
    {
        Self {
            timestamp: Utc::now().with_timezone(&Tz::UTC),
            summary: summary
                .map(|s| s.into())
                .unwrap_or_else(|| "sunny".to_string()),
            temp: 72.0,
            apparent_temp: None,
        }
    }
}

#[derive(Debug)]
pub struct WeatherForecast {
    pub current: Weather,
    pub upcoming: Vec<Weather>,
    pub timezone: Tz,
}

#[derive(Debug)]
pub enum WeatherProvider {
    AccuWeather,
    OpenWeather,
}

impl WeatherProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::AccuWeather => "accuweather",
            Self::OpenWeather => "openweather",
        }
    }

    pub async fn get_weather(
        &self,
        use_cache: bool,
        api_key: &str,
        latitude: f64,
        longitude: f64,
    ) -> Result<Vec<Weather>> {
        let weather = match self {
            Self::AccuWeather => {
                accu_weather::get_weather(use_cache, api_key, latitude, longitude).await
            }
            Self::OpenWeather => {
                open_weather::get_weather(use_cache, api_key, latitude, longitude).await
            }
        }?;
        debug!("{weather:?}");

        let now = Utc::now().with_timezone(&weather.timezone);

        let hours_of_interest = hours_of_interest(now, None, false);

        let mut filtered = Vec::with_capacity(1 + hours_of_interest.len());

        filtered.push(weather.current);

        for hourly_weather in weather.upcoming {
            if hourly_weather.timestamp.date_naive() > now.date_naive() {
                trace!("{:?} is no longer relevant", hourly_weather.timestamp);
                break;
            }

            if hourly_weather.timestamp.hour() == now.hour() {
                trace!("Skipping current hour: {:?}", hourly_weather.timestamp);
                continue;
            }

            if hours_of_interest.contains(&hourly_weather.timestamp.hour()) {
                debug!("{hourly_weather:?}");
                filtered.push(hourly_weather);
            }
        }

        Ok(filtered)
    }
}

impl FromStr for WeatherProvider {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if Self::AccuWeather.id().eq_ignore_ascii_case(s) {
            Ok(Self::AccuWeather)
        } else if Self::OpenWeather.id().eq_ignore_ascii_case(s) {
            Ok(Self::OpenWeather)
        } else {
            Err(anyhow!("Unknown weather provider: {}", s))
        }
    }
}

pub fn get_cache_path(weather_provider: &WeatherProvider, token: &str) -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!(
        "{}-{}-{token}.json",
        weather_provider.id(),
        Utc::now().date_naive().format("%Y%m%d"),
    ));

    path
}

pub async fn try_cached_query<F>(
    use_cache: bool,
    cache_path: &Path,
    query: impl Fn() -> F,
) -> Result<String>
where
    F: Future<Output = Result<String>>,
{
    match try_cached(use_cache, cache_path).await? {
        Some(cached) => Ok(cached),
        _ => {
            let response = query().await?;
            try_write_cache(use_cache, cache_path, &response).await?;
            Ok(response)
        }
    }
}

async fn try_cached(use_cache: bool, cache_path: &Path) -> Result<Option<String>> {
    if use_cache && cache_path.exists() {
        debug!("Reading cache file: {cache_path:?}");
        Ok(Some(fs::read_to_string(cache_path).await?))
    } else {
        Ok(None)
    }
}

async fn try_write_cache(use_cache: bool, cache_path: &Path, response: &str) -> Result<()> {
    if use_cache {
        debug!("Writing response to cache file: {cache_path:?}");

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

    debug!("Hours of Interest: {hours:?}");

    hours
}
