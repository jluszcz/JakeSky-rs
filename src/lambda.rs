use jakesky::weather::WeatherProvider;
use jakesky::{alexa, set_up_logger};
use lambda_runtime::{LambdaEvent, service_fn};
use log::{debug, info};
use serde_json::{Value, json};
use std::{env, error::Error};

type LambdaError = Box<dyn Error + Send + Sync + 'static>;

const RUSTC_VERSION: &str = env!("RUSTC_VERSION");

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

    info!("rustc version: {RUSTC_VERSION}");

    let api_key = env::var("JAKESKY_API_KEY")?;
    let latitude = env::var("JAKESKY_LATITUDE")?.parse()?;
    let longitude = env::var("JAKESKY_LONGITUDE")?.parse()?;

    let weather = WeatherProvider::AccuWeather
        .get_weather(false, &api_key, latitude, longitude)
        .await?;

    Ok(alexa::forecast(weather)?)
}
