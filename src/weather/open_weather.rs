use crate::weather::{self, Weather};
use anyhow::{anyhow, Result};
use chrono::serde::ts_seconds;
use chrono::{DateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use log::{debug, info, trace};
use reqwest::header::HeaderMap;
use serde::Deserialize;
use std::convert::{TryFrom, TryInto};
use std::str::FromStr;

#[derive(Deserialize, Debug)]
struct Response {
    timezone: String,
    current: WeatherItem,
    hourly: Vec<WeatherItem>,
}

#[derive(Deserialize, Debug)]
struct WeatherItem {
    #[serde(alias = "dt", with = "ts_seconds")]
    timestamp: DateTime<Utc>,

    weather: Vec<InnerWeather>,

    temp: f64,

    #[serde(alias = "feels_like", default)]
    apparent_temp: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct InnerWeather {
    main: String,
}

impl TryFrom<&(Tz, WeatherItem)> for Weather {
    type Error = anyhow::Error;

    fn try_from(value: &(Tz, WeatherItem)) -> Result<Self, Self::Error> {
        let (tz, weather) = value;

        let timestamp = tz.from_utc_datetime(&weather.timestamp.naive_utc());

        let summary = if weather.weather.len() != 1 {
            return Err(anyhow!(
                "Invalid number of weather items: {}",
                weather.weather.len()
            ));
        } else {
            let summary = &weather.weather[0].main;
            let summary = if summary.eq_ignore_ascii_case("Clouds") {
                "Cloudy"
            } else {
                summary
            };

            summary.to_string()
        };

        Ok(Self {
            timestamp,
            summary,
            temp: weather.temp,
            apparent_temp: weather.apparent_temp,
        })
    }
}

impl TryFrom<Response> for Vec<Weather> {
    type Error = anyhow::Error;

    fn try_from(response: Response) -> Result<Self, Self::Error> {
        let timezone = Tz::from_str(&response.timezone)
            .map_err(|_| anyhow!("Failed to parse timezone from {}", response.timezone))?;

        let now = timezone.from_utc_datetime(&response.current.timestamp.naive_utc());

        let hours_of_interest = weather::hours_of_interest(now, None, false);

        let mut weather = vec![Weather::try_from(&(timezone, response.current))?];

        for hourly_weather in response.hourly {
            let hourly_weather = Weather::try_from(&(timezone, hourly_weather))?;

            if hourly_weather.timestamp.date_naive() > now.date_naive() {
                debug!("{:?} is no longer relevant", hourly_weather.timestamp);
                break;
            }

            if hourly_weather.timestamp.hour() == now.hour() {
                debug!("Skipping current hour: {:?}", hourly_weather.timestamp);
                continue;
            }

            if hours_of_interest.contains(&hourly_weather.timestamp.hour()) {
                info!("{:?}", hourly_weather);
                weather.push(hourly_weather);
            }
        }

        Ok(weather)
    }
}

pub async fn query(open_weather_api_key: String, latitude: f64, longitude: f64) -> Result<String> {
    // Since we only care about the current and hourly forecast for specific times, exclude some of the data in the response.
    let url = format!(
      "https://api.openweathermap.org/data/2.5/onecall?exclude=minutely,daily,alerts&units=imperial&appid={}&lat={}&lon={}",
        open_weather_api_key, latitude, longitude
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
    let response: Response = serde_json::from_str(&response)?;
    response.try_into()
}

#[cfg(test)]
mod test {
    use super::*;

    const EXAMPLE_API_RESPONSE: &str = r#"{"lat":33.44,"lon":-94.04,"timezone":"America/Chicago","timezone_offset":-21600,"current":{"dt":1618317040,"sunrise":1618282134,"sunset":1618333901,"temp":284.07,"feels_like":282.84,"pressure":1019,"humidity":62,"dew_point":277.08,"uvi":0.89,"clouds":0,"visibility":10000,"wind_speed":6,"wind_deg":300,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10d"}],"rain":{"1h":0.21}},"minutely":[{"dt":1618317060,"precipitation":0.205}],"hourly":[{"dt":1618315200,"temp":282.58,"feels_like":280.4,"pressure":1019,"humidity":68,"dew_point":276.98,"uvi":1.4,"clouds":19,"visibility":306,"wind_speed":4.12,"wind_deg":296,"wind_gust":7.33,"weather":[{"id":801,"main":"Clouds","description":"few clouds","icon":"02d"}],"pop":0}],"daily":[{"dt":1618308000,"sunrise":1618282134,"sunset":1618333901,"moonrise":1618284960,"moonset":1618339740,"moon_phase":0.04,"temp":{"day":279.79,"min":275.09,"max":284.07,"night":275.09,"eve":279.21,"morn":278.49},"feels_like":{"day":277.59,"night":276.27,"eve":276.49,"morn":276.27},"pressure":1020,"humidity":81,"dew_point":276.77,"wind_speed":3.06,"wind_deg":294,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10d"}],"clouds":56,"pop":0.2,"rain":0.62,"uvi":1.93}],"alerts":[{"sender_name":"NWS Tulsa","event":"Heat Advisory","start":1597341600,"end":1597366800,"description":"...HEAT ADVISORY REMAINS IN EFFECT FROM 1 PM THIS AFTERNOON TO\n8 PM CDT THIS EVENING...\n* WHAT...Heat index values of 105 to 109 degrees expected.\n* WHERE...Creek, Okfuskee, Okmulgee, McIntosh, Pittsburg,\nLatimer, Pushmataha, and Choctaw Counties.\n* WHEN...From 1 PM to 8 PM CDT Thursday.\n* IMPACTS...The combination of hot temperatures and high\nhumidity will combine to create a dangerous situation in which\nheat illnesses are possible.","tags":["Extreme temperature value"]}]}"#;

    #[test]
    fn test_deserialize() -> Result<()> {
        let response = serde_json::from_str::<Response>(EXAMPLE_API_RESPONSE);

        assert!(response.is_ok());

        Ok(())
    }
}
