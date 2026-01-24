use crate::weather::{self, Weather, WeatherForecast, WeatherProvider};
use again::RetryPolicy;
use anyhow::{Context, Result, anyhow};
use chrono::serde::ts_seconds;
use chrono::{DateTime, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use log::{debug, trace};
use reqwest::Method;
use serde::Deserialize;
use std::convert::{TryFrom, TryInto};
use std::str::FromStr;
use std::time::Duration;

const WEATHER_PROVIDER: &WeatherProvider = &WeatherProvider::OpenWeather;

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

fn normalize_weather(weather: &str) -> String {
    if weather.eq_ignore_ascii_case("Clouds") {
        "Cloudy"
    } else {
        weather
    }
    .to_string()
}

impl TryFrom<&WeatherItem> for String {
    type Error = anyhow::Error;

    fn try_from(value: &WeatherItem) -> Result<Self, Self::Error> {
        if value.weather.is_empty() {
            return Err(anyhow!("Weather not found in {:?}", value));
        }

        let weather = value
            .weather
            .iter()
            .map(|w| normalize_weather(&w.main))
            .collect::<Vec<_>>();

        Ok(if weather.len() == 1 {
            weather.first().cloned().unwrap()
        } else {
            let (summary, second) = weather.split_at(weather.len() - 1);
            summary.join(", ") + " and " + &second[0]
        })
    }
}

impl TryFrom<&(Tz, WeatherItem)> for Weather {
    type Error = anyhow::Error;

    fn try_from(value: &(Tz, WeatherItem)) -> Result<Self, Self::Error> {
        let (tz, weather) = value;

        let timestamp = tz.from_utc_datetime(&weather.timestamp.naive_utc());
        let summary = weather
            .try_into()
            .with_context(|| "Failed to extract weather summary from OpenWeather data")?;

        Ok(Self {
            timestamp: timestamp.with_timezone(tz),
            summary,
            temp: weather.temp,
            apparent_temp: weather.apparent_temp,
        })
    }
}

impl TryFrom<Response> for Vec<Weather> {
    type Error = anyhow::Error;

    fn try_from(response: Response) -> Result<Self, Self::Error> {
        let timezone = Tz::from_str(&response.timezone).with_context(|| {
            format!(
                "Failed to parse timezone '{}' from OpenWeather API",
                response.timezone
            )
        })?;

        let now = timezone.from_utc_datetime(&response.current.timestamp.naive_utc());

        let mut weather = vec![
            Weather::try_from(&(timezone, response.current))
                .with_context(|| "Failed to parse current weather from OpenWeather")?,
        ];

        for (index, hourly_weather) in response.hourly.into_iter().enumerate() {
            let hourly_weather =
                Weather::try_from(&(timezone, hourly_weather)).with_context(|| {
                    format!("Failed to parse hourly weather entry {index} from OpenWeather")
                })?;

            if hourly_weather.timestamp.date_naive() > now.date_naive() {
                debug!("{:?} is no longer relevant", hourly_weather.timestamp);
                break;
            }

            if hourly_weather.timestamp.hour() == now.hour() {
                debug!("Skipping current hour: {:?}", hourly_weather.timestamp);
                continue;
            }

            weather.push(hourly_weather);
        }

        Ok(weather)
    }
}

pub async fn get_weather(
    use_cache: bool,
    api_key: &str,
    latitude: f64,
    longitude: f64,
) -> Result<WeatherForecast> {
    let cache_path =
        weather::get_cache_path(WEATHER_PROVIDER, &format!("{latitude:.1}_{longitude:.1}"));

    let response = weather::try_cached_query(use_cache, &cache_path, || {
        query(api_key, latitude, longitude)
    })
    .await
    .with_context(|| {
        format!(
            "Failed to get weather data from OpenWeather for coordinates {latitude}, {longitude}"
        )
    })?;

    let mut weather =
        parse_weather(response).with_context(|| "Failed to parse OpenWeather API response")?;

    Ok(WeatherForecast {
        timezone: weather[0].timestamp.timezone(),
        current: weather.remove(0),
        upcoming: weather,
    })
}

async fn query(api_key: &str, latitude: f64, longitude: f64) -> Result<String> {
    let retry_policy = RetryPolicy::exponential(Duration::from_millis(100))
        .with_jitter(true)
        .with_max_delay(Duration::from_secs(2))
        .with_max_retries(3);

    // Since we only care about the current and hourly forecast for specific times, exclude some of the data in the response.
    let response = retry_policy
        .retry(|| {
            weather::http_client()
                .request(
                    Method::GET,
                    "https://api.openweathermap.org/data/3.0/onecall",
                )
                .header("Accept", "application/json")
                .header("Accept-Encoding", "gzip")
                .query(&[
                    ("exclude", "minutely,daily,alerts"),
                    ("units", "imperial"),
                    ("appid", api_key),
                    ("lat", &format!("{latitude}")),
                    ("lon", &format!("{longitude}")),
                ])
                .send()
        })
        .await
        .with_context(|| "Failed to make HTTP request to OpenWeather API")?
        .error_for_status()
        .with_context(|| "OpenWeather API returned an error status")?
        .text()
        .await
        .with_context(|| "Failed to read OpenWeather API response body")?;

    trace!("{response}");

    Ok(response)
}

fn parse_weather(response: String) -> Result<Vec<Weather>> {
    let response: Response = serde_json::from_str(&response)
        .with_context(|| "Failed to deserialize JSON response from OpenWeather API")?;
    response.try_into()
}

#[cfg(test)]
mod test {
    use super::*;

    const EXAMPLE_API_RESPONSE: &str = r#"{"lat":42.341,"lon":-71.052,"timezone":"America/New_York","timezone_offset":-18000,"current":{"dt":1671203024,"sunrise":1671192428,"sunset":1671225160,"temp":42.53,"feels_like":32.11,"pressure":1012,"humidity":92,"dew_point":40.37,"uvi":0.1,"clouds":100,"visibility":4828,"wind_speed":28.77,"wind_deg":80,"wind_gust":35.68,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10d"},{"id":701,"main":"Mist","description":"mist","icon":"50d"}],"rain":{"1h":3.33}},"hourly":[{"dt":1671202800,"temp":42.53,"feels_like":34.07,"pressure":1012,"humidity":92,"dew_point":40.37,"uvi":0.1,"clouds":100,"visibility":7127,"wind_speed":18.86,"wind_deg":86,"wind_gust":31.61,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10d"}],"pop":1,"rain":{"1h":2.05}},{"dt":1671206400,"temp":42.3,"feels_like":33.62,"pressure":1011,"humidity":93,"dew_point":40.42,"uvi":0.13,"clouds":100,"visibility":4159,"wind_speed":19.42,"wind_deg":89,"wind_gust":33.08,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10d"}],"pop":1,"rain":{"1h":3.16}},{"dt":1671210000,"temp":41.86,"feels_like":33.48,"pressure":1011,"humidity":94,"dew_point":40.26,"uvi":0.14,"clouds":100,"visibility":2510,"wind_speed":17.76,"wind_deg":85,"wind_gust":31,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10d"}],"pop":1,"rain":{"1h":0.18}},{"dt":1671213600,"temp":41.47,"feels_like":32.56,"pressure":1009,"humidity":94,"dew_point":39.87,"uvi":0.11,"clouds":100,"visibility":5662,"wind_speed":19.37,"wind_deg":78,"wind_gust":34.07,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10d"}],"pop":1,"rain":{"1h":1.79}},{"dt":1671217200,"temp":41.49,"feels_like":32.4,"pressure":1007,"humidity":95,"dew_point":40.17,"uvi":0.08,"clouds":100,"visibility":6355,"wind_speed":20.15,"wind_deg":78,"wind_gust":34.14,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10d"}],"pop":1,"rain":{"1h":0.76}},{"dt":1671220800,"temp":41.34,"feels_like":32.23,"pressure":1005,"humidity":96,"dew_point":39.94,"uvi":0.03,"clouds":100,"visibility":2126,"wind_speed":20,"wind_deg":74,"wind_gust":34.4,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10d"}],"pop":1,"rain":{"1h":1.11}},{"dt":1671224400,"temp":41.65,"feels_like":32.7,"pressure":1004,"humidity":95,"dew_point":40.17,"uvi":0,"clouds":100,"visibility":3394,"wind_speed":19.73,"wind_deg":69,"wind_gust":35.41,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10d"}],"pop":1,"rain":{"1h":1.58}},{"dt":1671228000,"temp":42.49,"feels_like":33.6,"pressure":1003,"humidity":94,"dew_point":40.51,"uvi":0,"clouds":100,"visibility":7509,"wind_speed":20.69,"wind_deg":64,"wind_gust":37.31,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":2}},{"dt":1671231600,"temp":43.47,"feels_like":34.92,"pressure":1002,"humidity":93,"dew_point":41.2,"uvi":0,"clouds":100,"visibility":6519,"wind_speed":20.51,"wind_deg":64,"wind_gust":36.71,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":1.95}},{"dt":1671235200,"temp":44.04,"feels_like":36.25,"pressure":1002,"humidity":94,"dew_point":42.08,"uvi":0,"clouds":100,"visibility":6881,"wind_speed":17.96,"wind_deg":63,"wind_gust":33.6,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":2.84}},{"dt":1671238800,"temp":44.4,"feels_like":37.35,"pressure":1001,"humidity":94,"dew_point":42.48,"uvi":0,"clouds":100,"visibility":6058,"wind_speed":15.5,"wind_deg":59,"wind_gust":29.59,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":2.45}},{"dt":1671242400,"temp":44.6,"feels_like":37.83,"pressure":1000,"humidity":95,"dew_point":42.87,"uvi":0,"clouds":100,"visibility":7119,"wind_speed":14.63,"wind_deg":56,"wind_gust":29.19,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":2.26}},{"dt":1671246000,"temp":44.89,"feels_like":38.5,"pressure":999,"humidity":96,"dew_point":43.41,"uvi":0,"clouds":100,"visibility":8211,"wind_speed":13.6,"wind_deg":51,"wind_gust":27.83,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":1.97}},{"dt":1671249600,"temp":45.18,"feels_like":39.51,"pressure":997,"humidity":96,"dew_point":43.9,"uvi":0,"clouds":100,"visibility":7467,"wind_speed":11.56,"wind_deg":47,"wind_gust":24.81,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":1.75}},{"dt":1671253200,"temp":45.39,"feels_like":40.08,"pressure":996,"humidity":97,"dew_point":44.26,"uvi":0,"clouds":100,"visibility":7835,"wind_speed":10.65,"wind_deg":40,"wind_gust":23.29,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":1.51}},{"dt":1671256800,"temp":45.41,"feels_like":40.37,"pressure":995,"humidity":97,"dew_point":44.38,"uvi":0,"clouds":100,"visibility":6352,"wind_speed":9.95,"wind_deg":28,"wind_gust":22.21,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":1,"rain":{"1h":1.28}},{"dt":1671260400,"temp":45.23,"feels_like":39.51,"pressure":995,"humidity":96,"dew_point":43.83,"uvi":0,"clouds":100,"visibility":10000,"wind_speed":11.74,"wind_deg":12,"wind_gust":23,"weather":[{"id":501,"main":"Rain","description":"moderate rain","icon":"10n"}],"pop":0.89,"rain":{"1h":1.07}},{"dt":1671264000,"temp":44.4,"feels_like":37.31,"pressure":995,"humidity":93,"dew_point":42.33,"uvi":0,"clouds":100,"visibility":10000,"wind_speed":15.61,"wind_deg":2,"wind_gust":27.27,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10n"}],"pop":0.92,"rain":{"1h":0.48}},{"dt":1671267600,"temp":41.36,"feels_like":32.85,"pressure":996,"humidity":91,"dew_point":38.61,"uvi":0,"clouds":100,"visibility":10000,"wind_speed":17.67,"wind_deg":338,"wind_gust":30.58,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10n"}],"pop":0.92,"rain":{"1h":0.32}},{"dt":1671271200,"temp":37.58,"feels_like":28.31,"pressure":997,"humidity":94,"dew_point":35.65,"uvi":0,"clouds":100,"visibility":1773,"wind_speed":16.35,"wind_deg":310,"wind_gust":29.04,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10n"}],"pop":0.95,"rain":{"1h":0.38}},{"dt":1671274800,"temp":36.34,"feels_like":26.85,"pressure":997,"humidity":95,"dew_point":34.75,"uvi":0,"clouds":100,"visibility":72,"wind_speed":15.9,"wind_deg":299,"wind_gust":29.42,"weather":[{"id":601,"main":"Snow","description":"snow","icon":"13n"}],"pop":1,"snow":{"1h":0.54}},{"dt":1671278400,"temp":35.67,"feels_like":26.02,"pressure":998,"humidity":95,"dew_point":34.14,"uvi":0,"clouds":100,"visibility":94,"wind_speed":15.82,"wind_deg":301,"wind_gust":30.27,"weather":[{"id":601,"main":"Snow","description":"snow","icon":"13n"}],"pop":0.95,"snow":{"1h":0.56}},{"dt":1671282000,"temp":35.92,"feels_like":26.56,"pressure":999,"humidity":95,"dew_point":34.41,"uvi":0.02,"clouds":100,"visibility":163,"wind_speed":15.14,"wind_deg":304,"wind_gust":30.15,"weather":[{"id":600,"main":"Snow","description":"light snow","icon":"13d"}],"pop":0.7,"snow":{"1h":0.39}},{"dt":1671285600,"temp":36.93,"feels_like":28.27,"pressure":999,"humidity":94,"dew_point":35.2,"uvi":0.08,"clouds":100,"visibility":2452,"wind_speed":13.89,"wind_deg":300,"wind_gust":27.34,"weather":[{"id":600,"main":"Snow","description":"light snow","icon":"13d"}],"pop":0.7,"snow":{"1h":0.2}},{"dt":1671289200,"temp":37.6,"feels_like":29.52,"pressure":1000,"humidity":92,"dew_point":35.29,"uvi":0.15,"clouds":100,"visibility":6078,"wind_speed":12.8,"wind_deg":299,"wind_gust":25.14,"weather":[{"id":500,"main":"Rain","description":"light rain","icon":"10d"}],"pop":0.73,"rain":{"1h":0.19}},{"dt":1671292800,"temp":38.84,"feels_like":31.59,"pressure":999,"humidity":89,"dew_point":35.6,"uvi":0.53,"clouds":100,"visibility":8840,"wind_speed":11.48,"wind_deg":290,"wind_gust":21.88,"weather":[{"id":804,"main":"Clouds","description":"overcast clouds","icon":"04d"}],"pop":0.72},{"dt":1671296400,"temp":39.85,"feels_like":32.99,"pressure":999,"humidity":83,"dew_point":34.77,"uvi":0.56,"clouds":99,"visibility":10000,"wind_speed":11.14,"wind_deg":281,"wind_gust":20.78,"weather":[{"id":804,"main":"Clouds","description":"overcast clouds","icon":"04d"}],"pop":0.6},{"dt":1671300000,"temp":40.66,"feels_like":33.6,"pressure":999,"humidity":78,"dew_point":34.02,"uvi":0.47,"clouds":99,"visibility":10000,"wind_speed":12.19,"wind_deg":277,"wind_gust":20.87,"weather":[{"id":804,"main":"Clouds","description":"overcast clouds","icon":"04d"}],"pop":0.6},{"dt":1671303600,"temp":41.54,"feels_like":34.39,"pressure":1000,"humidity":74,"dew_point":33.46,"uvi":0.33,"clouds":92,"visibility":10000,"wind_speed":13.11,"wind_deg":274,"wind_gust":21.36,"weather":[{"id":804,"main":"Clouds","description":"overcast clouds","icon":"04d"}],"pop":0.22},{"dt":1671307200,"temp":40.57,"feels_like":33.22,"pressure":1000,"humidity":73,"dew_point":32.36,"uvi":0.13,"clouds":95,"visibility":10000,"wind_speed":12.95,"wind_deg":280,"wind_gust":22.48,"weather":[{"id":804,"main":"Clouds","description":"overcast clouds","icon":"04d"}],"pop":0.14},{"dt":1671310800,"temp":38.95,"feels_like":31.17,"pressure":1001,"humidity":73,"dew_point":30.94,"uvi":0,"clouds":73,"visibility":10000,"wind_speed":12.95,"wind_deg":280,"wind_gust":23.96,"weather":[{"id":803,"main":"Clouds","description":"broken clouds","icon":"04d"}],"pop":0.14},{"dt":1671314400,"temp":37.58,"feels_like":29.53,"pressure":1002,"humidity":74,"dew_point":29.93,"uvi":0,"clouds":62,"visibility":10000,"wind_speed":12.71,"wind_deg":278,"wind_gust":25.41,"weather":[{"id":803,"main":"Clouds","description":"broken clouds","icon":"04n"}],"pop":0.13},{"dt":1671318000,"temp":36.63,"feels_like":28.33,"pressure":1003,"humidity":76,"dew_point":29.39,"uvi":0,"clouds":53,"visibility":10000,"wind_speed":12.68,"wind_deg":274,"wind_gust":26.78,"weather":[{"id":803,"main":"Clouds","description":"broken clouds","icon":"04n"}],"pop":0.13},{"dt":1671321600,"temp":35.62,"feels_like":26.96,"pressure":1003,"humidity":77,"dew_point":28.83,"uvi":0,"clouds":52,"visibility":10000,"wind_speed":12.93,"wind_deg":269,"wind_gust":27.13,"weather":[{"id":803,"main":"Clouds","description":"broken clouds","icon":"04n"}],"pop":0.13},{"dt":1671325200,"temp":34.77,"feels_like":25.47,"pressure":1004,"humidity":78,"dew_point":28.26,"uvi":0,"clouds":57,"visibility":10000,"wind_speed":14.07,"wind_deg":268,"wind_gust":28.07,"weather":[{"id":803,"main":"Clouds","description":"broken clouds","icon":"04n"}],"pop":0},{"dt":1671328800,"temp":34.25,"feels_like":25.02,"pressure":1004,"humidity":76,"dew_point":27.14,"uvi":0,"clouds":54,"visibility":10000,"wind_speed":13.49,"wind_deg":273,"wind_gust":27.89,"weather":[{"id":803,"main":"Clouds","description":"broken clouds","icon":"04n"}],"pop":0},{"dt":1671332400,"temp":33.66,"feels_like":24.66,"pressure":1005,"humidity":75,"dew_point":26.28,"uvi":0,"clouds":41,"visibility":10000,"wind_speed":12.5,"wind_deg":274,"wind_gust":28.97,"weather":[{"id":802,"main":"Clouds","description":"scattered clouds","icon":"03n"}],"pop":0},{"dt":1671336000,"temp":33.13,"feels_like":24.28,"pressure":1005,"humidity":74,"dew_point":25.52,"uvi":0,"clouds":36,"visibility":10000,"wind_speed":11.86,"wind_deg":276,"wind_gust":27.94,"weather":[{"id":802,"main":"Clouds","description":"scattered clouds","icon":"03n"}],"pop":0},{"dt":1671339600,"temp":32.22,"feels_like":23.16,"pressure":1005,"humidity":75,"dew_point":24.93,"uvi":0,"clouds":31,"visibility":10000,"wind_speed":11.74,"wind_deg":271,"wind_gust":27.07,"weather":[{"id":802,"main":"Clouds","description":"scattered clouds","icon":"03n"}],"pop":0},{"dt":1671343200,"temp":31.77,"feels_like":22.5,"pressure":1005,"humidity":74,"dew_point":24.15,"uvi":0,"clouds":28,"visibility":10000,"wind_speed":11.97,"wind_deg":272,"wind_gust":27.76,"weather":[{"id":802,"main":"Clouds","description":"scattered clouds","icon":"03n"}],"pop":0},{"dt":1671346800,"temp":31.41,"feels_like":22.24,"pressure":1006,"humidity":73,"dew_point":23.58,"uvi":0,"clouds":26,"visibility":10000,"wind_speed":11.54,"wind_deg":269,"wind_gust":27.31,"weather":[{"id":802,"main":"Clouds","description":"scattered clouds","icon":"03n"}],"pop":0},{"dt":1671350400,"temp":31.33,"feels_like":22.21,"pressure":1006,"humidity":72,"dew_point":23.2,"uvi":0,"clouds":19,"visibility":10000,"wind_speed":11.41,"wind_deg":266,"wind_gust":27.49,"weather":[{"id":801,"main":"Clouds","description":"few clouds","icon":"02n"}],"pop":0},{"dt":1671354000,"temp":31.39,"feels_like":22.37,"pressure":1006,"humidity":71,"dew_point":22.93,"uvi":0,"clouds":18,"visibility":10000,"wind_speed":11.23,"wind_deg":264,"wind_gust":26.53,"weather":[{"id":801,"main":"Clouds","description":"few clouds","icon":"02n"}],"pop":0},{"dt":1671357600,"temp":31.26,"feels_like":22.44,"pressure":1007,"humidity":71,"dew_point":22.53,"uvi":0,"clouds":16,"visibility":10000,"wind_speed":10.74,"wind_deg":262,"wind_gust":26.13,"weather":[{"id":801,"main":"Clouds","description":"few clouds","icon":"02n"}],"pop":0},{"dt":1671361200,"temp":31.15,"feels_like":22.6,"pressure":1007,"humidity":70,"dew_point":22.12,"uvi":0,"clouds":15,"visibility":10000,"wind_speed":10.13,"wind_deg":259,"wind_gust":24.74,"weather":[{"id":801,"main":"Clouds","description":"few clouds","icon":"02n"}],"pop":0},{"dt":1671364800,"temp":30.97,"feels_like":22.41,"pressure":1007,"humidity":69,"dew_point":21.6,"uvi":0,"clouds":14,"visibility":10000,"wind_speed":10.09,"wind_deg":263,"wind_gust":24.47,"weather":[{"id":801,"main":"Clouds","description":"few clouds","icon":"02n"}],"pop":0},{"dt":1671368400,"temp":31.26,"feels_like":22.91,"pressure":1008,"humidity":66,"dew_point":20.95,"uvi":0.12,"clouds":11,"visibility":10000,"wind_speed":9.82,"wind_deg":263,"wind_gust":24.07,"weather":[{"id":801,"main":"Clouds","description":"few clouds","icon":"02d"}],"pop":0},{"dt":1671372000,"temp":32.63,"feels_like":24.08,"pressure":1008,"humidity":60,"dew_point":19.92,"uvi":0.39,"clouds":9,"visibility":10000,"wind_speed":10.89,"wind_deg":265,"wind_gust":22.73,"weather":[{"id":800,"main":"Clear","description":"clear sky","icon":"01d"}],"pop":0}]}
"#;

    #[test]
    fn test_deserialize() -> Result<()> {
        let response: Result<Response, _> = serde_json::from_str(EXAMPLE_API_RESPONSE);
        assert!(response.is_ok());

        let weathers: Result<Vec<Weather>> = response?.try_into();
        assert!(weathers.is_ok());

        Ok(())
    }
}
