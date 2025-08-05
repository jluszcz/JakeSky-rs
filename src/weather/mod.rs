use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use log::{debug, trace};
use reqwest::Client;
use std::env;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

pub mod accu_weather;
pub mod open_weather;

/// A secure wrapper for API keys that prevents accidental logging
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Creates a new ApiKey after basic validation
    pub fn new(key: impl Into<String>) -> Result<Self> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(anyhow!("API key cannot be empty"));
        }
        if key.len() < 8 {
            return Err(anyhow!(
                "API key appears to be too short (minimum 8 characters)"
            ));
        }
        Ok(Self(key))
    }

    /// Returns the API key as a string slice for use in API calls
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey([REDACTED])")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[REDACTED API KEY]")
    }
}

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
        api_key: &ApiKey,
        latitude: f64,
        longitude: f64,
    ) -> Result<Vec<Weather>> {
        // Validate coordinates before making API calls
        validate_coordinates(latitude, longitude)
            .with_context(|| format!("Invalid coordinates: lat={latitude}, lon={longitude}"))?;

        let weather = match self {
            Self::AccuWeather => {
                accu_weather::get_weather(use_cache, api_key.as_str(), latitude, longitude).await
            }
            Self::OpenWeather => {
                open_weather::get_weather(use_cache, api_key.as_str(), latitude, longitude).await
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
    // Sanitize token to remove special characters that could cause filesystem issues
    let sanitized_token = sanitize_filename(token);
    path.push(format!(
        "{}-{}-{}.json",
        weather_provider.id(),
        Utc::now().date_naive().format("%Y%m%d"),
        sanitized_token
    ));

    path
}

fn sanitize_filename(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            // Replace potentially problematic characters with underscores
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            // Keep dots for negative numbers but replace with 'n' prefix for clarity
            '.' => 'd',
            // Replace minus sign with 'n' for negative coordinates
            '-' => 'n',
            // Keep alphanumeric characters as-is
            c if c.is_alphanumeric() => c,
            // Replace any other special characters with underscores
            _ => '_',
        })
        .collect()
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
        Ok(Some(fs::read_to_string(cache_path).await.with_context(
            || format!("Failed to read cache file: {cache_path:?}"),
        )?))
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
            .await
            .with_context(|| format!("Failed to create or open cache file: {cache_path:?}"))?;

        file.write_all(response.as_bytes())
            .await
            .with_context(|| format!("Failed to write data to cache file: {cache_path:?}"))?;
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

/// Validates that latitude is within valid bounds (-90.0 to 90.0)
pub fn validate_latitude(latitude: f64) -> Result<()> {
    if !(-90.0..=90.0).contains(&latitude) {
        return Err(anyhow!(
            "Latitude must be between -90.0 and 90.0 degrees, got: {}",
            latitude
        ));
    }
    Ok(())
}

/// Validates that longitude is within valid bounds (-180.0 to 180.0)
pub fn validate_longitude(longitude: f64) -> Result<()> {
    if !(-180.0..=180.0).contains(&longitude) {
        return Err(anyhow!(
            "Longitude must be between -180.0 and 180.0 degrees, got: {}",
            longitude
        ));
    }
    Ok(())
}

/// Validates both latitude and longitude coordinates
pub fn validate_coordinates(latitude: f64, longitude: f64) -> Result<()> {
    validate_latitude(latitude).with_context(|| "Invalid latitude coordinate")?;
    validate_longitude(longitude).with_context(|| "Invalid longitude coordinate")?;
    Ok(())
}

// Shared HTTP client with optimized configuration
static HTTP_CLIENT: OnceLock<Client> = OnceLock::new();

pub fn http_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .gzip(true)
            .build()
            .expect("Failed to create HTTP client")
    })
}
