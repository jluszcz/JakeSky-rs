use anyhow::Result;
use clap::{Arg, ArgAction, Command};
use jakesky::weather::{ApiKey, WeatherProvider, validate_coordinates};
use jakesky::{APP_NAME, alexa};
use jluszcz_rust_utils::{Verbosity, set_up_logger};
use log::debug;
use std::str::FromStr;

#[derive(Debug)]
struct Args {
    verbosity: Verbosity,
    use_cache: bool,
    provider: WeatherProvider,
    api_key: ApiKey,
    latitude: f64,
    longitude: f64,
}

fn create_command() -> Command {
    Command::new("JakeSky-rs")
        .version("0.1")
        .author("Jacob Luszcz")
        .arg(
            Arg::new("verbosity")
                .short('v')
                .action(ArgAction::Count)
                .help("Increase verbosity (-v for debug, -vv for trace; max useful: -vv)"),
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
                .default_value("openweather")
                .help("Which weather provider to use"),
        )
}

fn args_from_matches(matches: clap::ArgMatches) -> Args {
    let verbosity = matches.get_count("verbosity").into();
    let use_cache = matches.get_flag("use-cache");
    let latitude = *matches.get_one::<f64>("latitude").unwrap();
    let longitude = *matches.get_one::<f64>("longitude").unwrap();
    let api_key = ApiKey::new(matches.get_one::<String>("api-key").cloned().unwrap()).unwrap();
    let provider = matches
        .get_one::<String>("provider")
        .and_then(|p| WeatherProvider::from_str(p).ok())
        .unwrap();

    Args {
        verbosity,
        use_cache,
        provider,
        api_key,
        latitude,
        longitude,
    }
}

fn parse_args() -> Args {
    let matches = create_command().get_matches();
    args_from_matches(matches)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();
    set_up_logger(APP_NAME, module_path!(), args.verbosity)?;
    debug!("{args:?}");

    // Validate coordinates early for better error messages
    validate_coordinates(args.latitude, args.longitude)?;

    let weather = args
        .provider
        .get_weather(args.use_cache, &args.api_key, args.latitude, args.longitude)
        .await?;

    alexa::forecast(weather)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> Vec<&'static str> {
        vec![
            "jakesky",
            "--latitude",
            "40.7128",
            "--longitude",
            "74.0060",
            "--api-key",
            "test-key",
        ]
    }

    // Test command without env var support for predictable testing
    fn create_test_command() -> Command {
        Command::new("JakeSky-rs")
            .version("0.1")
            .author("Jacob Luszcz")
            .arg(Arg::new("verbosity").short('v').action(ArgAction::Count))
            .arg(
                Arg::new("use-cache")
                    .short('c')
                    .long("cache")
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("latitude")
                    .long("latitude")
                    .alias("lat")
                    .required(true)
                    .value_parser(clap::value_parser!(f64)),
            )
            .arg(
                Arg::new("longitude")
                    .long("longitude")
                    .alias("long")
                    .required(true)
                    .value_parser(clap::value_parser!(f64)),
            )
            .arg(
                Arg::new("api-key")
                    .short('a')
                    .long("api-key")
                    .required(true),
            )
            .arg(
                Arg::new("provider")
                    .short('p')
                    .long("provider")
                    .value_parser([
                        WeatherProvider::AccuWeather.id(),
                        WeatherProvider::OpenWeather.id(),
                    ])
                    .default_value("openweather"),
            )
    }

    fn parse_args_from(args: &[&str]) -> Result<Args, clap::Error> {
        let matches = create_test_command().try_get_matches_from(args)?;
        Ok(args_from_matches(matches))
    }

    #[test]
    fn test_parse_args_minimal() {
        let args = parse_args_from(&base_args()).unwrap();

        assert!(matches!(args.verbosity, Verbosity::Info));
        assert!(!args.use_cache);
        assert_eq!(args.provider.id(), WeatherProvider::OpenWeather.id());
        assert_eq!(args.latitude, 40.7128);
        assert_eq!(args.longitude, 74.0060);
    }

    #[test]
    fn test_parse_args_with_verbosity() {
        let mut args = base_args();
        args.push("-vv");
        let args = parse_args_from(&args).unwrap();

        assert!(matches!(args.verbosity, Verbosity::Trace));
    }

    #[test]
    fn test_parse_args_with_cache() {
        let mut args = base_args();
        args.push("--cache");
        let args = parse_args_from(&args).unwrap();

        assert!(args.use_cache);
    }

    #[test]
    fn test_parse_args_with_provider() {
        let mut args = base_args();
        args.extend_from_slice(&["--provider", "accuweather"]);
        let args = parse_args_from(&args).unwrap();

        assert_eq!(args.provider.id(), WeatherProvider::AccuWeather.id());
    }

    #[test]
    fn test_parse_args_with_aliases() {
        let args = parse_args_from(&[
            "jakesky",
            "--lat",
            "40.7128",
            "--long",
            "74.0060",
            "-a",
            "test-key",
            "-p",
            "openweather",
        ])
        .unwrap();

        assert_eq!(args.latitude, 40.7128);
        assert_eq!(args.longitude, 74.0060);
        assert_eq!(args.provider.id(), WeatherProvider::OpenWeather.id());
    }

    #[test]
    fn test_parse_args_missing_required_latitude() {
        let result =
            parse_args_from(&["jakesky", "--longitude", "74.0060", "--api-key", "test-key"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_missing_required_longitude() {
        let result =
            parse_args_from(&["jakesky", "--latitude", "40.7128", "--api-key", "test-key"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_missing_required_api_key() {
        let result = parse_args_from(&[
            "jakesky",
            "--latitude",
            "40.7128",
            "--longitude",
            "-74.0060",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_invalid_provider() {
        let mut args = base_args();
        args.extend_from_slice(&["--provider", "invalid-provider"]);
        let result = parse_args_from(&args);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_invalid_latitude() {
        let result = parse_args_from(&[
            "jakesky",
            "--latitude",
            "invalid",
            "--longitude",
            "74.0060",
            "--api-key",
            "test-key",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_invalid_longitude() {
        let result = parse_args_from(&[
            "jakesky",
            "--latitude",
            "40.7128",
            "--longitude",
            "invalid",
            "--api-key",
            "test-key",
        ]);
        assert!(result.is_err());
    }
}
