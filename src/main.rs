use anyhow::Result;
use clap::{App, Arg};
use jakesky_rs::{alexa, dark_sky};
use lambda_runtime::{handler_fn, Context};
use log::{debug, info};
use serde_json::{json, Value};
use std::{env, error::Error};

type LambdaError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug)]
struct Args {
    verbose: bool,
    use_cache: bool,
    dark_sky_api_key: String,
    latitude: f64,
    longitude: f64,
}

fn parse_args() -> Args {
    let matches = App::new("JakeSky-rs")
        .version("0.1")
        .author("Jacob Luszcz")
        .arg(
            Arg::with_name("verbose")
                .short("v")
                .long("verbose")
                .help("Verbose mode. Outputs DEBUG and higher log messages."),
        )
        .arg(
            Arg::with_name("use-cache")
                .short("c")
                .long("cache")
                .help("Use cached values, if present, rather than querying remote services."),
        )
        .get_matches();

    let verbose = matches.is_present("verbose");

    let cache = matches.is_present("use-cache");

    let latitude = matches
        .value_of("latitude")
        .map(|l| l.into())
        .or_else(|| env::var("JAKESKY_LATITUDE").ok())
        .map(|l| l.parse().expect("Failed to parse latitude"))
        .expect("Missing latitude");

    let longitude = matches
        .value_of("longitude")
        .map(|l| l.into())
        .or_else(|| env::var("JAKESKY_LONGITUDE").ok())
        .map(|l| l.parse().expect("Failed to parse longitude"))
        .expect("Missing longitude");

    let dark_sky_api_key = matches
        .value_of("darksky-api")
        .map(|l| l.into())
        .or_else(|| env::var("JAKESKY_DARKSKY_KEY").ok())
        .expect("Missing Dark Sky API key");

    Args {
        verbose,
        use_cache: cache,
        dark_sky_api_key,
        latitude,
        longitude,
    }
}

fn setup_logger(verbose: bool) -> Result<()> {
    let level = if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    let _ = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{} [{}] [{}] {}",
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(level)
        .chain(std::io::stdout())
        .apply();

    Ok(())
}

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
    let args = parse_args();
    setup_logger(args.verbose)?;
    debug!("{:?}", args);

    if is_warmup_event(event) {
        info!("Warmup only, returning early");
        return Ok(json!({}));
    }

    let weather = dark_sky::get_weather_info(
        args.use_cache,
        args.dark_sky_api_key,
        args.latitude,
        args.longitude,
    )
    .await?;

    let forecast = alexa::forecast(weather)?;

    Ok(json!({
        "version": "1.0",
        "response": {
            "outputSpeech": {
                "type": "PlainText",
                "text": forecast,
            }
        }
    }))
}
