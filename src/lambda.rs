use jakesky::ai::BedrockSummarizer;
use jakesky::alert_summary;
use jakesky::weather::{ApiKey, WeatherProvider, validate_coordinates};
use jakesky::{APP_NAME, alexa};
use jluszcz_rust_utils::lambda;
use lambda_runtime::{LambdaEvent, service_fn};
use serde_json::{Value, json};
use std::env;

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
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

    let summarizer = if alert_summary::needs_llm_fallback(&alerts) {
        BedrockSummarizer::try_init().await
    } else {
        None
    };

    Ok(alexa::forecast(weather, alerts, summarizer.as_ref()).await?)
}
