use crate::config::Config;
use log::LevelFilter;
use simplelog::{self, ConfigBuilder};

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

        // Configure logging to suppress verbose logs from dependencies
        let log_config = ConfigBuilder::new()
            .add_filter_ignore_str("sqlx")
            .add_filter_ignore_str("sea_orm")
            .add_filter_ignore_str("tower")
            .add_filter_ignore_str("tracing")
            .add_filter_ignore_str("hyper")
            .build();

        simplelog::TermLogger::init(
            log_level_filter,
            log_config,
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        )
        .expect("Failed to start simplelog");
    }
}
