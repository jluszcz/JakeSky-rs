use jakesky::weather::{self, WeatherProvider};
use jakesky::{alexa, set_up_logger};
use lambda_runtime::{service_fn, LambdaEvent};
use log::debug;
use serde_json::{json, Value};
use std::{env, error::Error};

type LambdaError = Box<dyn Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() -> Result<(), LambdaError> {
    let func = service_fn(function);
    lambda_runtime::run(func).await?;
    Ok(())
}

fn is_warmup_event(event: Value) -> bool {
    "Scheduled Event"
        == event
            .get("detail-type")
            .and_then(|v| v.as_str())
            .unwrap_or("no detail-type")
}

async fn function(event: LambdaEvent<Value>) -> Result<Value, LambdaError> {
    set_up_logger(module_path!(), false)?;

    if is_warmup_event(event.payload) {
        debug!("Warmup only, returning early");
        return Ok(json!({}));
    }

    let api_key = env::var("JAKESKY_API_KEY")?;
    let latitude = env::var("JAKESKY_LATITUDE")?.parse()?;
    let longitude = env::var("JAKESKY_LONGITUDE")?.parse()?;

    let weather = weather::get_weather_info(
        &WeatherProvider::OpenWeather,
        false,
        api_key,
        latitude,
        longitude,
    )
    .await?;

    Ok(alexa::forecast(weather)?)
}
