use anyhow::Result;
use clap::{App, Arg};
use fern;
use log::{debug, info};

struct Args {
    verbose: bool,
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
        .get_matches();

    Args {
        verbose: matches.is_present("verbose"),
    }
}

fn setup_logger(verbose: bool) -> Result<()> {
    let level = if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    fern::Dispatch::new()
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
        .apply()?;

    Ok(())
}

fn main() -> Result<()> {
    let args = parse_args();
    setup_logger(args.verbose)?;

    debug!("Hello, verbose world!");
    info!("Hello, world!");

    Ok(())
}
