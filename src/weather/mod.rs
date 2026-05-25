use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use jluszcz_rust_utils::cache::CacheMode;
use log::{debug, trace};
use std::fmt;
use std::str::FromStr;

pub mod accu_weather;
pub mod open_weather;

/// Minimum length for an API key. Soft sanity check to catch obvious
/// configuration mistakes (e.g. empty or placeholder values), not a security
/// guarantee — real provider keys are far longer than this.
const MIN_API_KEY_LEN: usize = 8;

/// A secure wrapper for API keys that prevents accidental logging
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Creates a new ApiKey after sanity-checking the input.
    ///
    /// Rejects keys that are empty / whitespace-only, or shorter than
    /// [`MIN_API_KEY_LEN`] characters.
    pub fn new(key: impl Into<String>) -> Result<Self> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(anyhow!("API key cannot be empty"));
        }
        if key.len() < MIN_API_KEY_LEN {
            return Err(anyhow!(
                "API key appears to be too short (minimum {MIN_API_KEY_LEN} characters)"
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

#[derive(Debug, Clone)]
pub struct WeatherAlert {
    pub event: String,
    pub sender_name: String,
    pub start: DateTime<Tz>,
    pub end: DateTime<Tz>,
    pub description: String,
}

#[derive(Debug)]
pub struct WeatherForecast {
    pub current: Weather,
    pub upcoming: Vec<Weather>,
    pub timezone: Tz,
    pub alerts: Vec<WeatherAlert>,
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
        cache_mode: CacheMode,
        api_key: &ApiKey,
        latitude: f64,
        longitude: f64,
    ) -> Result<(Vec<Weather>, Vec<WeatherAlert>)> {
        validate_coordinates(latitude, longitude)
            .with_context(|| format!("Invalid coordinates: lat={latitude}, lon={longitude}"))?;

        let weather = match self {
            Self::AccuWeather => {
                accu_weather::get_weather(cache_mode, api_key, latitude, longitude).await
            }
            Self::OpenWeather => {
                open_weather::get_weather(cache_mode, api_key, latitude, longitude).await
            }
        }?;
        debug!("{weather:?}");

        let now = Utc::now().with_timezone(&weather.timezone);
        let alerts = weather.alerts;

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

        Ok((filtered, alerts))
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
fn validate_latitude(latitude: f64) -> Result<()> {
    if !(-90.0..=90.0).contains(&latitude) {
        return Err(anyhow!(
            "Latitude must be between -90.0 and 90.0 degrees, got: {}",
            latitude
        ));
    }
    Ok(())
}

/// Validates that longitude is within valid bounds (-180.0 to 180.0)
fn validate_longitude(longitude: f64) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "super-secret-key-value";

    #[test]
    fn api_key_debug_redacts_secret() {
        let key = ApiKey::new(SECRET).unwrap();
        let rendered = format!("{key:?}");
        assert!(
            !rendered.contains(SECRET),
            "Debug output leaked API key: {rendered}"
        );
    }

    #[test]
    fn api_key_display_redacts_secret() {
        let key = ApiKey::new(SECRET).unwrap();
        let rendered = format!("{key}");
        assert!(
            !rendered.contains(SECRET),
            "Display output leaked API key: {rendered}"
        );
    }

    #[test]
    fn api_key_as_str_returns_secret() {
        let key = ApiKey::new(SECRET).unwrap();
        assert_eq!(key.as_str(), SECRET);
    }
}
