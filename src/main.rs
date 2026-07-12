use anyhow::Result;
use clap::Parser;
use jakesky::ai;
use jakesky::weather::{ApiKey, WeatherProvider};
use jakesky::{APP_NAME, alexa};
use jluszcz_rust_utils::set_up_logger;
use log::debug;
use std::str::FromStr;

#[derive(Debug, Parser)]
#[command(name = "JakeSky-rs", version, author, infer_long_args = true)]
struct Args {
    /// Increase verbosity (-v for debug, -vv for trace; max useful: -vv)
    #[arg(short = 'v', action = clap::ArgAction::Count)]
    verbosity: u8,

    /// Use cached values, if present, rather than querying remote services.
    #[arg(short = 'c', long = "cache")]
    use_cache: bool,

    /// Latitude of location to get weather for
    #[arg(long, alias = "lat", env = "JAKESKY_LATITUDE", hide_env_values = true)]
    latitude: f64,

    /// Longitude of location to get weather for
    #[arg(
        long,
        alias = "long",
        env = "JAKESKY_LONGITUDE",
        hide_env_values = true
    )]
    longitude: f64,

    /// API key to use with the weather provider
    #[arg(
        short = 'a',
        long,
        env = "JAKESKY_API_KEY",
        hide_env_values = true,
        value_parser = parse_api_key
    )]
    api_key: ApiKey,

    /// Which weather provider to use (accuweather or openweather)
    #[arg(
        short = 'p',
        long,
        value_parser = parse_provider,
        default_value = WeatherProvider::OpenWeather.id()
    )]
    provider: WeatherProvider,
}

fn parse_api_key(s: &str) -> Result<ApiKey, String> {
    ApiKey::new(s).map_err(|e| e.to_string())
}

fn parse_provider(s: &str) -> Result<WeatherProvider, String> {
    WeatherProvider::from_str(s).map_err(|e| e.to_string())
}

fn parse_args() -> Args {
    Args::parse()
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
            args.use_cache.into(),
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
    use clap::{Command, CommandFactory, FromArgMatches};
    use jluszcz_rust_utils::Verbosity;
    use jluszcz_rust_utils::cache::CacheMode;

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
        ["latitude", "longitude", "api_key"]
            .into_iter()
            .fold(Args::command(), |command, name| {
                command.mut_arg(name, |arg| arg.env(None::<&'static str>))
            })
    }

    fn parse_args_from(args: &[&str]) -> Result<Args, clap::Error> {
        let matches = create_test_command().try_get_matches_from(args)?;
        Args::from_arg_matches(&matches)
    }

    #[test]
    fn test_parse_args_minimal() {
        let args = parse_args_from(&base_args()).unwrap();

        assert!(matches!(Verbosity::from(args.verbosity), Verbosity::Info));
        assert!(matches!(
            CacheMode::from(args.use_cache),
            CacheMode::Disabled
        ));
        assert_eq!(args.provider.id(), WeatherProvider::OpenWeather.id());
        assert_eq!(args.latitude, 40.7128);
        assert_eq!(args.longitude, 74.0060);
    }

    #[test]
    fn test_parse_args_with_verbosity() {
        let mut args = base_args();
        args.push("-vv");
        let args = parse_args_from(&args).unwrap();

        assert!(matches!(Verbosity::from(args.verbosity), Verbosity::Trace));
    }

    #[test]
    fn test_parse_args_with_cache() {
        let mut args = base_args();
        args.push("--cache");
        let args = parse_args_from(&args).unwrap();

        assert!(matches!(
            CacheMode::from(args.use_cache),
            CacheMode::Enabled
        ));
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
