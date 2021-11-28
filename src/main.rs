use anyhow::Result;
use clap::{App, Arg};
use jakesky::weather::{self, WeatherProvider};
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
        .arg(
            Arg::with_name("latitude")
                .long("latitude")
                .alias("lat")
                .required(true)
                .takes_value(true)
                .env("JAKESKY_LATITUDE")
                .hide_env_values(true)
                .help("Latitude of location to get weather for"),
        )
        .arg(
            Arg::with_name("longitude")
                .long("longitude")
                .alias("long")
                .required(true)
                .takes_value(true)
                .env("JAKESKY_LONGITUDE")
                .hide_env_values(true)
                .help("Longitude of location to get weather for"),
        )
        .arg(
            Arg::with_name("api-key")
                .short("a")
                .long("api-key")
                .required(true)
                .takes_value(true)
                .env("JAKESKY_API_KEY")
                .hide_env_values(true)
                .help("API key to use with the weather provider"),
        )
        .arg(
            Arg::with_name("provider")
                .short("p")
                .long("provider")
                .takes_value(true)
                .possible_values(&["darksky", "openweather"])
                .default_value("darksky")
                .help("Which weather provider to use"),
        )
        .get_matches();

    let verbose = matches.is_present("verbose");

    let cache = matches.is_present("use-cache");

    let latitude = matches
        .value_of("latitude")
        .map(|l| l.parse().expect("Failed to parse latitude"))
        .unwrap();

    let longitude = matches
        .value_of("longitude")
        .map(|l| l.parse().expect("Failed to parse longitude"))
        .unwrap();

    let api_key = matches.value_of("api-key").map(|l| l.into()).unwrap();

    let provider = matches
        .value_of("provider")
        .map(|l| {
            if "darksky".eq_ignore_ascii_case(l) {
                Some(WeatherProvider::DarkSky)
            } else if "openweather".eq_ignore_ascii_case(l) {
                Some(WeatherProvider::OpenWeather)
            } else {
                None
            }
        })
        .flatten()
        .unwrap();

    Args {
        verbose,
        use_cache: cache,
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

    let weather = weather::get_weather_info(
        &args.provider,
        args.use_cache,
        args.api_key,
        args.latitude,
        args.longitude,
    )
    .await?;

    alexa::forecast(weather)?;

    Ok(())
}
