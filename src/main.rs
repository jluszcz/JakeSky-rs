use anyhow::Result;
use clap::{App, Arg};
use jakesky_rs::weather::{self, WeatherProvider};
use jakesky_rs::{alexa, set_up_logger};
use log::debug;
use std::env;

#[derive(Debug)]
struct Args {
    verbose: bool,
    use_cache: bool,
    api_key: String,
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

    let api_key = matches
        .value_of("api-key")
        .map(|l| l.into())
        .or_else(|| env::var("JAKESKY_API_KEY").ok())
        .expect("Missing Weather API key");

    Args {
        verbose,
        use_cache: cache,
        api_key,
        latitude,
        longitude,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    set_up_logger(args.verbose)?;
    debug!("{:?}", args);

    let weather = weather::get_weather_info(
        &WeatherProvider::DarkSky,
        args.use_cache,
        args.api_key,
        args.latitude,
        args.longitude,
    )
    .await?;

    alexa::forecast(weather)?;

    Ok(())
}
