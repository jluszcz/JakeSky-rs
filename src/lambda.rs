use jakesky::weather::{ApiKey, WeatherProvider, validate_coordinates};
use jakesky::{APP_NAME, alexa};
use jluszcz_rust_utils::lambda;
use lambda_runtime::{LambdaEvent, service_fn};
use serde_json::{Value, json};
use std::env;

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
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

async fn function(event: LambdaEvent<Value>) -> Result<Value, lambda_runtime::Error> {
    if is_warmup_event(event.payload) {
        return Ok(json!({}));
    }

    lambda::init(APP_NAME, module_path!(), false).await?;

    let api_key = ApiKey::new(env::var("JAKESKY_API_KEY")?)?;
    let latitude = env::var("JAKESKY_LATITUDE")?.parse()?;
    let longitude = env::var("JAKESKY_LONGITUDE")?.parse()?;

    // Validate coordinates
    validate_coordinates(latitude, longitude)?;

    let (weather, alerts) = WeatherProvider::OpenWeather
        .get_weather(false, &api_key, latitude, longitude)
        .await?;

    Ok(alexa::forecast(weather, alerts)?)
}
