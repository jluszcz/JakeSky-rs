use std::str::FromStr;

use anyhow::Result;
use clap::{Arg, ArgAction, Command};
use jakesky::weather::WeatherProvider;
use jakesky::{alexa, set_up_logger};
use log::debug;

#[derive(Debug)]
struct Args {
    verbose: bool,
    use_cache: bool,
    provider: WeatherProvider,
    api_key: String,
    latitude: f64,
    longitude: f64,
}

fn parse_args() -> Args {
    let matches = Command::new("JakeSky-rs")
        .version("0.1")
        .author("Jacob Luszcz")
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::SetTrue)
                .help("Verbose mode. Outputs DEBUG and higher log messages."),
        )
        .arg(
            Arg::new("use-cache")
                .short('c')
                .long("cache")
                .action(ArgAction::SetTrue)
                .help("Use cached values, if present, rather than querying remote services."),
        )
        .arg(
            Arg::new("latitude")
                .long("latitude")
                .alias("lat")
                .required(true)
                .env("JAKESKY_LATITUDE")
                .hide_env_values(true)
                .value_parser(clap::value_parser!(f64))
                .help("Latitude of location to get weather for"),
        )
        .arg(
            Arg::new("longitude")
                .long("longitude")
                .alias("long")
                .required(true)
                .env("JAKESKY_LONGITUDE")
                .hide_env_values(true)
                .value_parser(clap::value_parser!(f64))
                .help("Longitude of location to get weather for"),
        )
        .arg(
            Arg::new("api-key")
                .short('a')
                .long("api-key")
                .required(true)
                .env("JAKESKY_API_KEY")
                .hide_env_values(true)
                .help("API key to use with the weather provider"),
        )
        .arg(
            Arg::new("provider")
                .short('p')
                .long("provider")
                .value_parser([
                    WeatherProvider::AccuWeather.id(),
                    WeatherProvider::OpenWeather.id(),
                ])
                .default_value("accuweather")
                .help("Which weather provider to use"),
        )
        .get_matches();

    let verbose = matches.get_flag("verbose");

    let use_cache = matches.get_flag("use-cache");

    let latitude = *matches.get_one::<f64>("latitude").unwrap();

    let longitude = *matches.get_one::<f64>("longitude").unwrap();

    let api_key = matches
        .get_one::<String>("api-key")
        .map(|l| l.into())
        .unwrap();

    let provider = matches
        .get_one::<String>("provider")
        .and_then(|p| WeatherProvider::from_str(p).ok())
        .unwrap();

    Args {
        verbose,
        use_cache,
        provider,
        api_key,
        latitude,
        longitude,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    set_up_logger(module_path!(), args.verbose)?;
    debug!("{:?}", args);

    let weather = args
        .provider
        .get_weather(args.use_cache, &args.api_key, args.latitude, args.longitude)
        .await?;

    alexa::forecast(weather)?;

    Ok(())
}
