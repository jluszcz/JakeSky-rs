use anyhow::Result;
use clap::{Arg, ArgAction, Command};
use jakesky::ai;
use jakesky::weather::{ApiKey, WeatherProvider};
use jakesky::{APP_NAME, alexa};
use jluszcz_rust_utils::cache::CacheMode;
use jluszcz_rust_utils::{Verbosity, set_up_logger};
use log::debug;
use std::str::FromStr;

#[derive(Debug)]
struct Args {
    verbosity: Verbosity,
    cache_mode: CacheMode,
    provider: WeatherProvider,
    api_key: ApiKey,
    latitude: f64,
    longitude: f64,
}

fn create_command() -> Command {
    Command::new("JakeSky-rs")
        .version(env!("CARGO_PKG_VERSION"))
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
                .value_parser(parse_api_key)
                .help("API key to use with the weather provider"),
        )
        .arg(
            Arg::new("provider")
                .short('p')
                .long("provider")
                .value_parser(parse_provider)
                .default_value(WeatherProvider::OpenWeather.id())
                .help("Which weather provider to use (accuweather or openweather)"),
        )
}

fn parse_api_key(s: &str) -> Result<ApiKey, String> {
    ApiKey::new(s).map_err(|e| e.to_string())
}

fn parse_provider(s: &str) -> Result<WeatherProvider, String> {
    WeatherProvider::from_str(s).map_err(|e| e.to_string())
}

fn args_from_matches(matches: clap::ArgMatches) -> Args {
    let verbosity = matches.get_count("verbosity").into();
    let cache_mode = matches.get_flag("use-cache").into();
    let latitude = *matches.get_one::<f64>("latitude").unwrap();
    let longitude = *matches.get_one::<f64>("longitude").unwrap();
    let api_key = matches.get_one::<ApiKey>("api-key").cloned().unwrap();
    let provider = *matches.get_one::<WeatherProvider>("provider").unwrap();

    Args {
        verbosity,
        cache_mode,
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
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let args = parse_args();
    set_up_logger(APP_NAME, module_path!(), args.verbosity)?;
    debug!("{args:?}");

    let report = args
        .provider
        .get_weather(
            args.cache_mode,
            &args.api_key,
            args.latitude,
            args.longitude,
        )
        .await?;

    let summarizer = ai::summarizer_for(&report.alerts).await;

    alexa::forecast(report.weather, report.alerts, summarizer.as_ref()).await?;

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

    /// The real command with env-var support stripped, so ambient JAKESKY_*
    /// variables can't leak into tests.
    fn create_test_command() -> Command {
        ["latitude", "longitude", "api-key"]
            .into_iter()
            .fold(create_command(), |command, name| {
                command.mut_arg(name, |arg| arg.env(None::<&'static str>))
            })
    }

    fn parse_args_from(args: &[&str]) -> Result<Args, clap::Error> {
        let matches = create_test_command().try_get_matches_from(args)?;
        Ok(args_from_matches(matches))
    }

    #[test]
    fn test_parse_args_minimal() {
        let args = parse_args_from(&base_args()).unwrap();

        assert!(matches!(args.verbosity, Verbosity::Info));
        assert!(matches!(args.cache_mode, CacheMode::Disabled));
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

        assert!(matches!(args.cache_mode, CacheMode::Enabled));
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
