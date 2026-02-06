use crate::config::Config;
use log::LevelFilter;
use simplelog::{self, ConfigBuilder};

/// Modules to filter out from logging when not in Trace mode.
/// These are typically verbose dependencies that clutter normal log output.
const FILTERED_MODULES: &[&str] = &["sqlx", "sea_orm", "tower", "tracing", "hyper", "axum"];

pub struct Logger {}

impl Logger {
    /// Initializes the global logger with configuration based on the provided Config.
    ///
    /// When the log level is set to Trace, all logs including dependency logs are shown.
    /// For all other log levels, verbose dependency logs are filtered out.
    pub fn init_logger(config: &Config) {
        let log_level_filter = Self::convert_level_filter(config.log_level_filter);
        let apply_filters = Self::should_filter_dependencies(config.log_level_filter);
        let log_config = Self::build_log_config(apply_filters);

        simplelog::TermLogger::init(
            log_level_filter,
            log_config,
            simplelog::TerminalMode::Mixed,
            simplelog::ColorChoice::Auto,
        )
        .expect("Failed to start simplelog");
    }

    /// Converts log::LevelFilter to simplelog::LevelFilter.
    fn convert_level_filter(level: LevelFilter) -> simplelog::LevelFilter {
        match level {
            LevelFilter::Off => simplelog::LevelFilter::Off,
            LevelFilter::Error => simplelog::LevelFilter::Error,
            LevelFilter::Warn => simplelog::LevelFilter::Warn,
            LevelFilter::Info => simplelog::LevelFilter::Info,
            LevelFilter::Debug => simplelog::LevelFilter::Debug,
            LevelFilter::Trace => simplelog::LevelFilter::Trace,
        }
    }

    /// Determines whether dependency logging should be filtered.
    ///
    /// Returns `false` for Trace level (show all logs), `true` for all other levels.
    fn should_filter_dependencies(level: LevelFilter) -> bool {
        level != LevelFilter::Trace
    }

    /// Builds a simplelog Config with optional module filtering.
    ///
    /// When `apply_filters` is true, logs from noisy dependencies are suppressed.
    fn build_log_config(apply_filters: bool) -> simplelog::Config {
        let mut builder = ConfigBuilder::new();
        builder.set_time_format_rfc3339();

        if apply_filters {
            for module in FILTERED_MODULES {
                builder.add_filter_ignore_str(module);
            }
        }

        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filtered_modules_contains_expected_dependencies() {
        // Verify all expected noisy dependencies are in the filter list
        assert!(
            FILTERED_MODULES.contains(&"sqlx"),
            "sqlx should be filtered"
        );
        assert!(
            FILTERED_MODULES.contains(&"sea_orm"),
            "sea_orm should be filtered"
        );
        assert!(
            FILTERED_MODULES.contains(&"tower"),
            "tower should be filtered"
        );
        assert!(
            FILTERED_MODULES.contains(&"tracing"),
            "tracing should be filtered"
        );
        assert!(
            FILTERED_MODULES.contains(&"hyper"),
            "hyper should be filtered"
        );
        assert!(
            FILTERED_MODULES.contains(&"axum"),
            "axum should be filtered"
        );
    }

    #[test]
    fn test_should_filter_dependencies_trace_level_disables_filtering() {
        // Trace level should NOT filter - we want to see everything for deep debugging
        assert!(
            !Logger::should_filter_dependencies(LevelFilter::Trace),
            "Trace level should disable filtering"
        );
    }

    #[test]
    fn test_should_filter_dependencies_other_levels_enable_filtering() {
        // All other levels should filter out noisy dependencies
        assert!(
            Logger::should_filter_dependencies(LevelFilter::Off),
            "Off level should enable filtering"
        );
        assert!(
            Logger::should_filter_dependencies(LevelFilter::Error),
            "Error level should enable filtering"
        );
        assert!(
            Logger::should_filter_dependencies(LevelFilter::Warn),
            "Warn level should enable filtering"
        );
        assert!(
            Logger::should_filter_dependencies(LevelFilter::Info),
            "Info level should enable filtering"
        );
        assert!(
            Logger::should_filter_dependencies(LevelFilter::Debug),
            "Debug level should enable filtering"
        );
    }

    #[test]
    fn test_build_log_config_with_filters_does_not_panic() {
        // Verify building config with filters doesn't panic
        let _config = Logger::build_log_config(true);
    }

    #[test]
    fn test_build_log_config_without_filters_does_not_panic() {
        // Verify building config without filters doesn't panic
        let _config = Logger::build_log_config(false);
    }

    #[test]
    fn test_convert_level_filter_all_variants() {
        // Verify all level filter conversions work correctly
        assert_eq!(
            Logger::convert_level_filter(LevelFilter::Off) as u8,
            simplelog::LevelFilter::Off as u8
        );
        assert_eq!(
            Logger::convert_level_filter(LevelFilter::Error) as u8,
            simplelog::LevelFilter::Error as u8
        );
        assert_eq!(
            Logger::convert_level_filter(LevelFilter::Warn) as u8,
            simplelog::LevelFilter::Warn as u8
        );
        assert_eq!(
            Logger::convert_level_filter(LevelFilter::Info) as u8,
            simplelog::LevelFilter::Info as u8
        );
        assert_eq!(
            Logger::convert_level_filter(LevelFilter::Debug) as u8,
            simplelog::LevelFilter::Debug as u8
        );
        assert_eq!(
            Logger::convert_level_filter(LevelFilter::Trace) as u8,
            simplelog::LevelFilter::Trace as u8
        );
    }
}
