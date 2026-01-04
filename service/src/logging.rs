use crate::config::Config;
use log::LevelFilter;
use simplelog;

pub struct Logger {}

impl Logger {
    pub fn init_logger(config: &Config) {
        let log_level_filter = match config.log_level_filter {
            LevelFilter::Off => simplelog::LevelFilter::Off,
            LevelFilter::Error => simplelog::LevelFilter::Error,
            LevelFilter::Warn => simplelog::LevelFilter::Warn,
            LevelFilter::Info => simplelog::LevelFilter::Info,
            LevelFilter::Debug => simplelog::LevelFilter::Debug,
            LevelFilter::Trace => simplelog::LevelFilter::Trace,
        };

        simplelog::TermLogger::init(
            log_level_filter,
            simplelog::Config::default(),
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        )
        .expect("Failed to start simplelog");
    }
}
