use std::borrow::Cow;

use anyhow::Result;
use log::LevelFilter;

pub mod alexa;
pub mod weather;

pub fn set_up_logger<T>(calling_module: T, verbose: bool) -> Result<()>
where
    T: Into<Cow<'static, str>>,
{
    let level = if verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
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
        .level(LevelFilter::Warn)
        .level_for("jakesky", level)
        .level_for(calling_module, level)
        .chain(std::io::stdout())
        .apply();

    Ok(())
}
