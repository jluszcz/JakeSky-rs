#![recursion_limit = "256"]

use anyhow::Context;
use jakesky::ai;
use jakesky::weather::{ApiKey, WeatherProvider};
use jakesky::{APP_NAME, alexa};
use jluszcz_rust_utils::cache::CacheMode;
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

fn is_warmup_event(event: &Value) -> bool {
    "Scheduled Event"
        == event
            .get("detail-type")
            .and_then(|v| v.as_str())
            .unwrap_or("no detail-type")
}

async fn function(event: LambdaEvent<Value>) -> Result<Value, lambda_runtime::Error> {
    if is_warmup_event(&event.payload) {
        return Ok(json!({}));
    }

    lambda::init(APP_NAME, module_path!(), false).await?;

    let api_key =
        ApiKey::new(env::var("JAKESKY_API_KEY")?).context("JAKESKY_API_KEY is invalid")?;
    let latitude = env::var("JAKESKY_LATITUDE")?.parse()?;
    let longitude = env::var("JAKESKY_LONGITUDE")?.parse()?;

    let report = WeatherProvider::OpenWeather
        .get_weather(CacheMode::Disabled, &api_key, latitude, longitude)
        .await?;

    let summarizer = ai::summarizer_for(&report.alerts).await;

    Ok(alexa::forecast(report.weather, report.alerts, summarizer.as_ref()).await?)
}
