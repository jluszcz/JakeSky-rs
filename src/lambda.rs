use jakesky_rs::{alexa, dark_sky, set_up_logger};
use lambda_runtime::{handler_fn, Context};
use log::debug;
use serde_json::{json, Value};
use std::{env, error::Error};

type LambdaError = Box<dyn Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() -> Result<(), LambdaError> {
    let func = handler_fn(function);
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

async fn function(event: Value, _: Context) -> Result<Value, LambdaError> {
    set_up_logger(false)?;

    if is_warmup_event(event) {
        debug!("Warmup only, returning early");
        return Ok(json!({}));
    }

    let dark_sky_api_key = env::var("JAKESKY_DARKSKY_KEY")?;
    let latitude = env::var("JAKESKY_LATITUDE")?.parse()?;
    let longitude = env::var("JAKESKY_LONGITUDE")?.parse()?;

    let weather = dark_sky::get_weather_info(false, dark_sky_api_key, latitude, longitude).await?;

    Ok(alexa::forecast(weather)?)
}
