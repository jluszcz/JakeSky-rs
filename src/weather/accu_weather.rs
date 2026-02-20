use crate::weather::{self, Weather, WeatherForecast};
use again::RetryPolicy;
use anyhow::{Context, Result, anyhow};
use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use jluszcz_rust_utils::cache::{dated_cache_path, try_cached_query};
use log::trace;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;

#[derive(Deserialize, Debug)]
struct LocationResponse {
    #[serde(alias = "Key")]
    id: String,

    #[serde(alias = "TimeZone")]
    timezone: TimeZone,
}

#[derive(Deserialize, Debug)]
struct TimeZone {
    #[serde(alias = "Name")]
    name: String,
}

#[derive(Deserialize, Debug)]
struct CurrentConditionsResponse {
    #[serde(alias = "EpochTime", with = "ts_seconds")]
    timestamp: DateTime<Utc>,

    #[serde(alias = "WeatherText")]
    weather: String,

    #[serde(alias = "Temperature")]
    temp: ImperialTemperature,

    #[serde(default, alias = "RealFeelTemperature")]
    feels_like_temp: Option<ImperialTemperature>,
}

#[derive(Deserialize, Debug)]
struct ImperialTemperature {
    #[serde(alias = "Imperial")]
    imperial: Temperature,
}

#[derive(Deserialize, Debug)]
struct WeatherResponse {
    #[serde(alias = "EpochDateTime", with = "ts_seconds")]
    timestamp: DateTime<Utc>,

    #[serde(alias = "IconPhrase")]
    weather: String,

    #[serde(alias = "Temperature")]
    temp: Temperature,

    #[serde(default, alias = "RealFeelTemperature")]
    feels_like_temp: Option<Temperature>,
}

#[derive(Deserialize, Debug)]
struct Temperature {
    #[serde(alias = "Value")]
    value: f64,
}

impl TryFrom<(CurrentConditionsResponse, &str)> for Weather {
    type Error = anyhow::Error;

    fn try_from(value: (CurrentConditionsResponse, &str)) -> Result<Self, Self::Error> {
        let (curr, timezone) = value;
        let timezone = Tz::from_str(timezone).with_context(|| {
            format!("Failed to parse timezone '{timezone}' from AccuWeather response")
        })?;

        Ok(Self {
            timestamp: curr.timestamp.with_timezone(&timezone),
            summary: normalize_weather(&curr.weather),
            temp: curr.temp.imperial.value,
            apparent_temp: curr.feels_like_temp.map(|t| t.imperial.value),
        })
    }
}

impl TryFrom<(WeatherResponse, &str)> for Weather {
    type Error = anyhow::Error;

    fn try_from(value: (WeatherResponse, &str)) -> Result<Self, Self::Error> {
        let (weather, timezone) = value;
        let timezone = Tz::from_str(timezone).with_context(|| {
            format!("Failed to parse timezone '{timezone}' from AccuWeather forecast response")
        })?;

        Ok(Self {
            timestamp: weather.timestamp.with_timezone(&timezone),
            summary: normalize_weather(&weather.weather),
            temp: weather.temp.value,
            apparent_temp: weather.feels_like_temp.map(|f| f.value),
        })
    }
}

fn normalize_weather(weather: &str) -> String {
    weather
        .replace("w/", "with")
        .replace("t-storms", "thunderstorms")
}

async fn http_get<T>(url: &str, params: &T) -> Result<String>
where
    T: Serialize + ?Sized,
{
    let retry_policy = RetryPolicy::exponential(Duration::from_millis(100))
        .with_jitter(true)
        .with_max_delay(Duration::from_secs(2))
        .with_max_retries(3);

    let response = retry_policy
        .retry(|| {
            weather::http_client()
                .request(Method::GET, url)
                .header("Accept", "application/json")
                .header("Accept-Encoding", "gzip")
                .query(params)
                .send()
        })
        .await
        .with_context(|| format!("Failed to make HTTP request to {url}"))?
        .error_for_status()
        .with_context(|| format!("HTTP request failed for {url}"))?
        .text()
        .await
        .with_context(|| "Failed to read response body")?;

    trace!("{response}");

    Ok(response)
}

async fn query_location(api_key: &str, latitude: f64, longitude: f64) -> Result<String> {
    http_get(
        "http://dataservice.accuweather.com/locations/v1/cities/geoposition/search",
        &[
            ("apikey", api_key),
            ("q", &format!("{latitude},{longitude}")),
        ],
    )
    .await
}

async fn query_current_conditions(api_key: &str, location_id: &str) -> Result<String> {
    http_get(
        &format!("http://dataservice.accuweather.com/currentconditions/v1/{location_id}"),
        &[("apikey", api_key), ("details", "true")],
    )
    .await
}

async fn query_weather(api_key: &str, location_id: &str) -> Result<String> {
    http_get(
        &format!("http://dataservice.accuweather.com/forecasts/v1/hourly/12hour/{location_id}"),
        &[("apikey", api_key), ("details", "true")],
    )
    .await
}

pub async fn get_weather(
    use_cache: bool,
    api_key: &str,
    latitude: f64,
    longitude: f64,
) -> Result<WeatherForecast> {
    let token_suffix = format!("{latitude:.1}_{longitude:.1}");

    let location_cache_path = dated_cache_path(&format!("accuweather-location_{token_suffix}"));
    let weather_cache_path = dated_cache_path(&format!("accuweather-weather_{token_suffix}"));
    let current_conditions_cache_path =
        dated_cache_path(&format!("accuweather-curr_{token_suffix}"));

    let location = try_cached_query(use_cache, &location_cache_path, || {
        query_location(api_key, latitude, longitude)
    })
    .await
    .with_context(|| {
        format!("Failed to get location data for coordinates {latitude}, {longitude}")
    })?;

    let location: LocationResponse = serde_json::from_str(&location)
        .with_context(|| "Failed to parse location response from AccuWeather API")?;

    let current_conditions = try_cached_query(use_cache, &current_conditions_cache_path, || {
        query_current_conditions(api_key, &location.id)
    })
    .await
    .with_context(|| {
        format!(
            "Failed to get current conditions for location ID {}",
            location.id
        )
    })?;

    let weather_data = try_cached_query(use_cache, &weather_cache_path, || {
        query_weather(api_key, &location.id)
    })
    .await
    .with_context(|| {
        format!(
            "Failed to get weather forecast for location ID {}",
            location.id
        )
    })?;

    let current = parse_current_conditions(&current_conditions, &location.timezone.name)
        .with_context(|| "Failed to parse current weather conditions")?;
    let upcoming = parse_weather(&weather_data, &location.timezone.name)
        .with_context(|| "Failed to parse weather forecast data")?;

    Ok(WeatherForecast {
        timezone: current.timestamp.timezone(),
        current,
        upcoming,
        alerts: Vec::new(), // AccuWeather alerts are not currently implemented
    })
}

fn parse_current_conditions(response: &str, timezone: &str) -> Result<Weather> {
    let response: Vec<CurrentConditionsResponse> = serde_json::from_str(response)
        .with_context(|| "Failed to deserialize current conditions JSON from AccuWeather")?;
    let response = response
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("AccuWeather API returned empty current conditions array"))?;
    (response, timezone).try_into()
}

fn parse_weather(response: &str, timezone: &str) -> Result<Vec<Weather>> {
    let response: Vec<WeatherResponse> = serde_json::from_str(response)
        .with_context(|| "Failed to deserialize weather forecast JSON from AccuWeather")?;

    let mut weather = Vec::new();
    for (index, w) in response.into_iter().enumerate() {
        weather.push((w, timezone).try_into().with_context(|| {
            format!("Failed to convert weather entry {index} from AccuWeather")
        })?);
    }

    Ok(weather)
}

#[cfg(test)]
mod test {
    use super::*;

    const LOCATION_RESPONSE: &str = r#"{"Version":1,"Key":"2627484","Type":"City","Rank":55,"LocalizedName":"Midtown","EnglishName":"Midtown","PrimaryPostalCode":"10022","Region":{"ID":"NAM","LocalizedName":"North America","EnglishName":"North America"},"Country":{"ID":"US","LocalizedName":"United States","EnglishName":"United States"},"AdministrativeArea":{"ID":"NY","LocalizedName":"New York","EnglishName":"New York","Level":1,"LocalizedType":"State","EnglishType":"State","CountryID":"US"},"TimeZone":{"Code":"EDT","Name":"America/New_York","GmtOffset":-4,"IsDaylightSaving":true,"NextOffsetChange":"2023-11-05T06:00:00Z"},"GeoPosition":{"Latitude":40.759,"Longitude":-73.976,"Elevation":{"Metric":{"Value":25,"Unit":"m","UnitType":5},"Imperial":{"Value":82,"Unit":"ft","UnitType":0}}},"IsAlias":false,"ParentCity":{"Key":"349727","LocalizedName":"New York","EnglishName":"New York"},"SupplementalAdminAreas":[{"Level":2,"LocalizedName":"New York","EnglishName":"New York"}],"DataSets":["AirQualityCurrentConditions","AirQualityForecasts","Alerts","DailyAirQualityForecast","DailyPollenForecast","ForecastConfidence","FutureRadar","MinuteCast","Radar"]}"#;

    const CURRENT_CONDITIONS_RESPONSE: &str = r#"[{"LocalObservationDateTime":"2023-03-19T09:38:00-04:00","EpochTime":1679233080,"WeatherText":"Sunny","WeatherIcon":1,"HasPrecipitation":false,"PrecipitationType":null,"IsDayTime":true,"Temperature":{"Metric":{"Value":-1.1,"Unit":"C","UnitType":17},"Imperial":{"Value":30,"Unit":"F","UnitType":18}},"RealFeelTemperature":{"Metric":{"Value":0.4,"Unit":"C","UnitType":17,"Phrase":"Cold"},"Imperial":{"Value":33,"Unit":"F","UnitType":18,"Phrase":"Cold"}},"RealFeelTemperatureShade":{"Metric":{"Value":-3.1,"Unit":"C","UnitType":17,"Phrase":"Cold"},"Imperial":{"Value":26,"Unit":"F","UnitType":18,"Phrase":"Cold"}},"RelativeHumidity":32,"IndoorRelativeHumidity":19,"DewPoint":{"Metric":{"Value":-15.6,"Unit":"C","UnitType":17},"Imperial":{"Value":4,"Unit":"F","UnitType":18}},"Wind":{"Direction":{"Degrees":0,"Localized":"N","English":"N"},"Speed":{"Metric":{"Value":9.3,"Unit":"km/h","UnitType":7},"Imperial":{"Value":5.8,"Unit":"mi/h","UnitType":9}}},"WindGust":{"Speed":{"Metric":{"Value":35.2,"Unit":"km/h","UnitType":7},"Imperial":{"Value":21.9,"Unit":"mi/h","UnitType":9}}},"UVIndex":3,"UVIndexText":"Moderate","Visibility":{"Metric":{"Value":16.1,"Unit":"km","UnitType":6},"Imperial":{"Value":10,"Unit":"mi","UnitType":2}},"ObstructionsToVisibility":"","CloudCover":0,"Ceiling":{"Metric":{"Value":12192,"Unit":"m","UnitType":5},"Imperial":{"Value":40000,"Unit":"ft","UnitType":0}},"Pressure":{"Metric":{"Value":1015.7,"Unit":"mb","UnitType":14},"Imperial":{"Value":29.99,"Unit":"inHg","UnitType":12}},"PressureTendency":{"LocalizedText":"Rising","Code":"R"},"Past24HourTemperatureDeparture":{"Metric":{"Value":-7.2,"Unit":"C","UnitType":17},"Imperial":{"Value":-13,"Unit":"F","UnitType":18}},"ApparentTemperature":{"Metric":{"Value":-1.1,"Unit":"C","UnitType":17},"Imperial":{"Value":30,"Unit":"F","UnitType":18}},"WindChillTemperature":{"Metric":{"Value":-4.4,"Unit":"C","UnitType":17},"Imperial":{"Value":24,"Unit":"F","UnitType":18}},"WetBulbTemperature":{"Metric":{"Value":-4.9,"Unit":"C","UnitType":17},"Imperial":{"Value":23,"Unit":"F","UnitType":18}},"Precip1hr":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"PrecipitationSummary":{"Precipitation":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"PastHour":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"Past3Hours":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"Past6Hours":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"Past9Hours":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"Past12Hours":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"Past18Hours":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}},"Past24Hours":{"Metric":{"Value":0,"Unit":"mm","UnitType":3},"Imperial":{"Value":0,"Unit":"in","UnitType":1}}},"TemperatureSummary":{"Past6HourRange":{"Minimum":{"Metric":{"Value":-1.1,"Unit":"C","UnitType":17},"Imperial":{"Value":30,"Unit":"F","UnitType":18}},"Maximum":{"Metric":{"Value":2.8,"Unit":"C","UnitType":17},"Imperial":{"Value":37,"Unit":"F","UnitType":18}}},"Past12HourRange":{"Minimum":{"Metric":{"Value":-1.1,"Unit":"C","UnitType":17},"Imperial":{"Value":30,"Unit":"F","UnitType":18}},"Maximum":{"Metric":{"Value":8.3,"Unit":"C","UnitType":17},"Imperial":{"Value":47,"Unit":"F","UnitType":18}}},"Past24HourRange":{"Minimum":{"Metric":{"Value":-1.1,"Unit":"C","UnitType":17},"Imperial":{"Value":30,"Unit":"F","UnitType":18}},"Maximum":{"Metric":{"Value":11.7,"Unit":"C","UnitType":17},"Imperial":{"Value":53,"Unit":"F","UnitType":18}}}},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/current-weather/2627484?lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/current-weather/2627484?lang=en-us"}]"#;

    const WEATHER_RESPONSE: &str = r#"[{"DateTime":"2023-03-19T09:00:00-04:00","EpochDateTime":1679230800,"WeatherIcon":1,"IconPhrase":"Sunny","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":32,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":25,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":23,"Unit":"F","UnitType":18,"Phrase":"Very Cold"},"WetBulbTemperature":{"Value":24,"Unit":"F","UnitType":18},"DewPoint":{"Value":3,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":9,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":271,"Localized":"W","English":"W"}},"WindGust":{"Speed":{"Value":21,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":271,"Localized":"W","English":"W"}},"RelativeHumidity":29,"IndoorRelativeHumidity":18,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":33000,"Unit":"ft","UnitType":0},"UVIndex":1,"UVIndexText":"Low","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":5,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":394.39,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=9&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=9&lang=en-us"},{"DateTime":"2023-03-19T10:00:00-04:00","EpochDateTime":1679234400,"WeatherIcon":2,"IconPhrase":"Mostly sunny","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":33,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":29,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":25,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":25,"Unit":"F","UnitType":18},"DewPoint":{"Value":4,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":9,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":272,"Localized":"W","English":"W"}},"WindGust":{"Speed":{"Value":21,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":272,"Localized":"W","English":"W"}},"RelativeHumidity":29,"IndoorRelativeHumidity":19,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":2,"UVIndexText":"Low","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":10,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":523.57,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=10&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=10&lang=en-us"},{"DateTime":"2023-03-19T11:00:00-04:00","EpochDateTime":1679238000,"WeatherIcon":2,"IconPhrase":"Mostly sunny","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":35,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":30,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":24,"Unit":"F","UnitType":18,"Phrase":"Very Cold"},"WetBulbTemperature":{"Value":27,"Unit":"F","UnitType":18},"DewPoint":{"Value":7,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":12,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":275,"Localized":"W","English":"W"}},"WindGust":{"Speed":{"Value":23,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":275,"Localized":"W","English":"W"}},"RelativeHumidity":30,"IndoorRelativeHumidity":21,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":4,"UVIndexText":"Moderate","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":10,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":627.43,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=11&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=11&lang=en-us"},{"DateTime":"2023-03-19T12:00:00-04:00","EpochDateTime":1679241600,"WeatherIcon":2,"IconPhrase":"Mostly sunny","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":37,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":32,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":26,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":28,"Unit":"F","UnitType":18},"DewPoint":{"Value":6,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":13,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":277,"Localized":"W","English":"W"}},"WindGust":{"Speed":{"Value":27,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":277,"Localized":"W","English":"W"}},"RelativeHumidity":28,"IndoorRelativeHumidity":20,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":4,"UVIndexText":"Moderate","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":10,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":691.88,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=12&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=12&lang=en-us"},{"DateTime":"2023-03-19T13:00:00-04:00","EpochDateTime":1679245200,"WeatherIcon":2,"IconPhrase":"Mostly sunny","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":39,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":33,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":27,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":29,"Unit":"F","UnitType":18},"DewPoint":{"Value":7,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":15,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":276,"Localized":"W","English":"W"}},"WindGust":{"Speed":{"Value":29,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":276,"Localized":"W","English":"W"}},"RelativeHumidity":26,"IndoorRelativeHumidity":21,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":5,"UVIndexText":"Moderate","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":10,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":716.92,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=13&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=13&lang=en-us"},{"DateTime":"2023-03-19T14:00:00-04:00","EpochDateTime":1679248800,"WeatherIcon":4,"IconPhrase":"Intermittent clouds","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":40,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":33,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":28,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":30,"Unit":"F","UnitType":18},"DewPoint":{"Value":7,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":15,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":279,"Localized":"W","English":"W"}},"WindGust":{"Speed":{"Value":29,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":279,"Localized":"W","English":"W"}},"RelativeHumidity":25,"IndoorRelativeHumidity":21,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":4,"UVIndexText":"Moderate","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":59,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":447.42,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=14&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=14&lang=en-us"},{"DateTime":"2023-03-19T15:00:00-04:00","EpochDateTime":1679252400,"WeatherIcon":4,"IconPhrase":"Intermittent clouds","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":40,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":33,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":29,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":30,"Unit":"F","UnitType":18},"DewPoint":{"Value":7,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":16,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":284,"Localized":"WNW","English":"WNW"}},"WindGust":{"Speed":{"Value":29,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":284,"Localized":"WNW","English":"WNW"}},"RelativeHumidity":25,"IndoorRelativeHumidity":21,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":3,"UVIndexText":"Moderate","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":66,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":379.51,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=15&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=15&lang=en-us"},{"DateTime":"2023-03-19T16:00:00-04:00","EpochDateTime":1679256000,"WeatherIcon":4,"IconPhrase":"Intermittent clouds","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":42,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":33,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":31,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":31,"Unit":"F","UnitType":18},"DewPoint":{"Value":7,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":16,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":286,"Localized":"WNW","English":"WNW"}},"WindGust":{"Speed":{"Value":29,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":286,"Localized":"WNW","English":"WNW"}},"RelativeHumidity":23,"IndoorRelativeHumidity":21,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":2,"UVIndexText":"Low","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":67,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":320.88,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=16&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=16&lang=en-us"},{"DateTime":"2023-03-19T17:00:00-04:00","EpochDateTime":1679259600,"WeatherIcon":4,"IconPhrase":"Intermittent clouds","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":41,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":31,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":29,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":30,"Unit":"F","UnitType":18},"DewPoint":{"Value":7,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":16,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":288,"Localized":"WNW","English":"WNW"}},"WindGust":{"Speed":{"Value":29,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":288,"Localized":"WNW","English":"WNW"}},"RelativeHumidity":24,"IndoorRelativeHumidity":21,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":1,"UVIndexText":"Low","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":58,"Evapotranspiration":{"Value":0.01,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":272.51,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=17&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=17&lang=en-us"},{"DateTime":"2023-03-19T18:00:00-04:00","EpochDateTime":1679263200,"WeatherIcon":3,"IconPhrase":"Partly sunny","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":40,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":30,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":30,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":30,"Unit":"F","UnitType":18},"DewPoint":{"Value":7,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":14,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":291,"Localized":"WNW","English":"WNW"}},"WindGust":{"Speed":{"Value":25,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":291,"Localized":"WNW","English":"WNW"}},"RelativeHumidity":25,"IndoorRelativeHumidity":21,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":0,"UVIndexText":"Low","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":46,"Evapotranspiration":{"Value":0,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":184.01,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=18&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=18&lang=en-us"},{"DateTime":"2023-03-19T19:00:00-04:00","EpochDateTime":1679266800,"WeatherIcon":3,"IconPhrase":"Partly sunny","HasPrecipitation":false,"IsDaylight":true,"Temperature":{"Value":39,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":30,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":30,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":30,"Unit":"F","UnitType":18},"DewPoint":{"Value":8,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":12,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":295,"Localized":"WNW","English":"WNW"}},"WindGust":{"Speed":{"Value":22,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":295,"Localized":"WNW","English":"WNW"}},"RelativeHumidity":28,"IndoorRelativeHumidity":22,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":0,"UVIndexText":"Low","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":46,"Evapotranspiration":{"Value":0,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":29,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=19&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=19&lang=en-us"},{"DateTime":"2023-03-19T20:00:00-04:00","EpochDateTime":1679270400,"WeatherIcon":33,"IconPhrase":"Clear","HasPrecipitation":false,"IsDaylight":false,"Temperature":{"Value":39,"Unit":"F","UnitType":18},"RealFeelTemperature":{"Value":31,"Unit":"F","UnitType":18,"Phrase":"Cold"},"RealFeelTemperatureShade":{"Value":31,"Unit":"F","UnitType":18,"Phrase":"Cold"},"WetBulbTemperature":{"Value":30,"Unit":"F","UnitType":18},"DewPoint":{"Value":10,"Unit":"F","UnitType":18},"Wind":{"Speed":{"Value":10,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":300,"Localized":"WNW","English":"WNW"}},"WindGust":{"Speed":{"Value":18,"Unit":"mi/h","UnitType":9},"Direction":{"Degrees":300,"Localized":"WNW","English":"WNW"}},"RelativeHumidity":30,"IndoorRelativeHumidity":24,"Visibility":{"Value":10,"Unit":"mi","UnitType":2},"Ceiling":{"Value":30000,"Unit":"ft","UnitType":0},"UVIndex":0,"UVIndexText":"Low","PrecipitationProbability":0,"ThunderstormProbability":0,"RainProbability":0,"SnowProbability":0,"IceProbability":0,"TotalLiquid":{"Value":0,"Unit":"in","UnitType":1},"Rain":{"Value":0,"Unit":"in","UnitType":1},"Snow":{"Value":0,"Unit":"in","UnitType":1},"Ice":{"Value":0,"Unit":"in","UnitType":1},"CloudCover":0,"Evapotranspiration":{"Value":0,"Unit":"in","UnitType":1},"SolarIrradiance":{"Value":0,"Unit":"W/m²","UnitType":33},"MobileLink":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=20&lang=en-us","Link":"http://www.accuweather.com/en/us/midtown-ny/10022/hourly-weather-forecast/2627484?day=1&hbhhour=20&lang=en-us"}]"#;

    #[test]
    pub fn test_deserialize_location_response() -> Result<()> {
        let location_response: LocationResponse = serde_json::from_str(LOCATION_RESPONSE)?;

        assert_eq!("2627484", location_response.id);
        assert_eq!("America/New_York", location_response.timezone.name);

        Ok(())
    }

    #[test]
    pub fn test_deserialize_current_conditions_response() -> Result<()> {
        let current_conditions_response: Vec<CurrentConditionsResponse> =
            serde_json::from_str(CURRENT_CONDITIONS_RESPONSE)?;

        assert_eq!("Sunny", current_conditions_response[0].weather);
        assert_eq!(30.0, current_conditions_response[0].temp.imperial.value);

        assert!(current_conditions_response[0].feels_like_temp.is_some());
        assert_eq!(
            33.0,
            current_conditions_response[0]
                .feels_like_temp
                .as_ref()
                .unwrap()
                .imperial
                .value
        );

        Ok(())
    }

    #[test]
    pub fn test_deserialize_weather_response() -> Result<()> {
        let location_response: Vec<WeatherResponse> = serde_json::from_str(WEATHER_RESPONSE)?;

        assert_eq!(12, location_response.len());
        assert_eq!("Sunny", location_response[0].weather);
        assert_eq!(32.0, location_response[0].temp.value);

        assert!(location_response[0].feels_like_temp.is_some());
        assert_eq!(
            25.0,
            location_response[0].feels_like_temp.as_ref().unwrap().value
        );

        Ok(())
    }
}
