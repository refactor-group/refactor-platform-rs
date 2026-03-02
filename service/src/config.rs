use clap::builder::TypedValueParser as _;
use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches, Parser};
use dotenvy::dotenv;
use log::{debug, warn, LevelFilter};
use semver::{BuildMetadata, Prerelease, Version};
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use utoipa::IntoParams;

type APiVersionList = [&'static str; 1];

const DEFAULT_API_VERSION: &str = "1.0.0-beta1";
// Expand this array to include all valid API versions. Versions that have been
// completely removed should be removed from this list - they're no longer valid.
const API_VERSIONS: APiVersionList = [DEFAULT_API_VERSION];

static X_VERSION: &str = "x-version";

/// Default MailerSend API base URL used when `MAILERSEND_BASE_URL` is not set or empty.
pub const DEFAULT_MAILERSEND_BASE_URL: &str = "https://api.mailersend.com/v1";

/// Default URL path for session-scheduled email links.
const DEFAULT_SESSION_SCHEDULED_EMAIL_URL_PATH: &str = "/coaching-sessions/{session_id}";

/// Default URL path for action-assigned email links.
const DEFAULT_ACTION_ASSIGNED_EMAIL_URL_PATH: &str = "/coaching-sessions/{session_id}?tab=actions";

/// All config field names registered with Clap, used for value source tracking.
/// This is the single source of truth for field key names across the Config type.
const CONFIG_FIELD_KEYS: &[&str] = &[
    "allowed_origins",
    "api_version",
    "database_url",
    "db_max_connections",
    "db_min_connections",
    "db_connect_timeout_secs",
    "db_acquire_timeout_secs",
    "db_idle_timeout_secs",
    "db_max_lifetime_secs",
    "tiptap_url",
    "tiptap_auth_key",
    "tiptap_jwt_signing_key",
    "tiptap_app_id",
    "mailersend_base_url",
    "mailersend_api_key",
    "welcome_email_template_id",
    "session_scheduled_email_template_id",
    "action_assigned_email_template_id",
    "frontend_base_url",
    "session_scheduled_email_url_path",
    "action_assigned_email_url_path",
    "interface",
    "port",
    "log_level_filter",
    "runtime_env",
    "backend_session_expiry_seconds",
];

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Header)]
pub struct ApiVersion {
    /// The version of the API to use for a request.
    #[param(rename = "x-version", style = Simple, required, example = "1.0.0-beta1")]
    pub version: Version,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RustEnv {
    Development,
    Production,
    Staging,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RustEnvParseError;

impl FromStr for RustEnv {
    type Err = RustEnvParseError;
    fn from_str(level: &str) -> Result<RustEnv, Self::Err> {
        match level.to_lowercase().as_str() {
            "development" => Ok(RustEnv::Development),
            "production" => Ok(RustEnv::Production),
            "staging" => Ok(RustEnv::Staging),
            _ => Err(RustEnvParseError),
        }
    }
}

impl fmt::Display for RustEnv {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RustEnv::Development => write!(f, "development"),
            RustEnv::Production => write!(f, "production"),
            RustEnv::Staging => write!(f, "staging"),
        }
    }
}

/// Trait for formatting config values in debug log output.
/// Provides a unified interface so `Config::log_field` can accept any field type.
trait ConfigDisplay {
    fn display_value(&self) -> String;
}

impl ConfigDisplay for String {
    fn display_value(&self) -> String {
        self.clone()
    }
}

impl ConfigDisplay for u16 {
    fn display_value(&self) -> String {
        self.to_string()
    }
}

impl ConfigDisplay for u32 {
    fn display_value(&self) -> String {
        self.to_string()
    }
}

impl ConfigDisplay for u64 {
    fn display_value(&self) -> String {
        self.to_string()
    }
}

impl ConfigDisplay for Vec<String> {
    fn display_value(&self) -> String {
        format!("{:?}", self)
    }
}

impl ConfigDisplay for LevelFilter {
    fn display_value(&self) -> String {
        self.to_string()
    }
}

impl ConfigDisplay for RustEnv {
    fn display_value(&self) -> String {
        format!("{:?}", self)
    }
}

impl<T: ConfigDisplay> ConfigDisplay for Option<T> {
    fn display_value(&self) -> String {
        match self {
            Some(v) => v.display_value(),
            None => "None".to_string(),
        }
    }
}

#[derive(Clone, Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// A list of full CORS origin URLs that allowed to receive server responses.
    #[arg(
        long,
        env,
        value_delimiter = ',',
        use_value_delimiter = true,
        default_value = "http://localhost:3000,https://localhost:3000"
    )]
    pub allowed_origins: Vec<String>,

    /// Set the current semantic version of the endpoint API to expose to clients. All
    /// endpoints not contained in the specified version will not be exposed by the router.
    #[arg(short, long, env, default_value = DEFAULT_API_VERSION,
        value_parser = clap::builder::PossibleValuesParser::new(API_VERSIONS)
            .map(|s| s.parse::<String>().unwrap()),
        )]
    pub api_version: Option<String>,

    /// Sets the Postgresql database URL to connect to
    #[arg(
        short,
        long,
        env,
        default_value = "postgres://refactor:password@localhost:5432/refactor"
    )]
    database_url: Option<String>,

    /// Maximum number of database connections in the pool
    #[arg(long, env, default_value_t = 100)]
    pub db_max_connections: u32,

    /// Minimum number of idle database connections to maintain
    #[arg(long, env, default_value_t = 5)]
    pub db_min_connections: u32,

    /// Timeout in seconds for establishing a new database connection
    #[arg(long, env, default_value_t = 8)]
    pub db_connect_timeout_secs: u64,

    /// Timeout in seconds for acquiring a connection from the pool
    #[arg(long, env, default_value_t = 8)]
    pub db_acquire_timeout_secs: u64,

    /// Seconds before an idle connection is closed
    #[arg(long, env, default_value_t = 600)]
    pub db_idle_timeout_secs: u64,

    /// Maximum lifetime in seconds for any connection in the pool
    #[arg(long, env, default_value_t = 1800)]
    pub db_max_lifetime_secs: u64,

    /// The URL for the Tiptap Cloud API provider
    #[arg(long, env)]
    tiptap_url: Option<String>,

    /// The authorization key to use when calling the Tiptap Cloud API.
    #[arg(long, env)]
    tiptap_auth_key: Option<String>,

    /// The signing key to use when calling the Tiptap Cloud API.
    #[arg(long, env)]
    tiptap_jwt_signing_key: Option<String>,

    /// The application ID to use when calling the Tiptap Cloud API.
    #[arg(long, env)]
    tiptap_app_id: Option<String>,

    /// The base URL of the MailerSend API.
    /// Override in tests to point at a mock server.
    #[arg(long, env, default_value = DEFAULT_MAILERSEND_BASE_URL)]
    mailersend_base_url: String,
    /// The API key to use when calling the MailerSend API.
    #[arg(long, env)]
    mailersend_api_key: Option<String>,
    /// The MailerSend template ID for welcome emails.
    #[arg(long, env)]
    welcome_email_template_id: Option<String>,
    /// The MailerSend template ID for session-scheduled emails.
    #[arg(long, env)]
    session_scheduled_email_template_id: Option<String>,
    /// The MailerSend template ID for action-assigned emails.
    #[arg(long, env)]
    action_assigned_email_template_id: Option<String>,
    /// The base URL of the frontend application (e.g. https://app.myrefactor.com).
    /// Used to construct links in email notifications.
    #[arg(long, env)]
    frontend_base_url: Option<String>,
    /// URL path template for session-scheduled email links.
    /// Use `{session_id}` as a placeholder for the coaching session ID.
    #[arg(long, env, default_value = "/coaching-sessions/{session_id}")]
    session_scheduled_email_url_path: String,
    /// URL path template for action-assigned email links.
    /// Use `{session_id}` as a placeholder for the coaching session ID.
    #[arg(
        long,
        env,
        default_value = "/coaching-sessions/{session_id}?tab=actions"
    )]
    action_assigned_email_url_path: String,

    /// The host interface to listen for incoming connections
    #[arg(short, long, env, default_value = "127.0.0.1")]
    pub interface: Option<String>,

    /// The host TCP port to listen for incoming connections
    #[arg(short, long, env, default_value_t = 4000)]
    pub port: u16,

    /// Set the log level verbosity threshold (level) to control what gets displayed on console output
    #[arg(
        short,
        long,
        env,
        default_value_t = LevelFilter::Info,
        value_parser = clap::builder::PossibleValuesParser::new(["OFF", "ERROR", "WARN", "INFO", "DEBUG", "TRACE"])
            .map(|s| s.parse::<LevelFilter>().unwrap()),
        )]
    pub log_level_filter: LevelFilter,

    /// Set the Rust runtime environment to use.
    #[arg(
    short,
    long,
    env,
    default_value_t = RustEnv::Development,
    value_parser = clap::builder::PossibleValuesParser::new([
        "DEVELOPMENT", "PRODUCTION", "STAGING", 
        "development", "production", "staging"
    ])
        .map(|s| s.parse::<RustEnv>().unwrap()),
    )]
    pub runtime_env: RustEnv,

    /// Session expiry duration in seconds (default: 24 hours = 86400 seconds)
    #[arg(long, env, default_value_t = 86400)]
    pub backend_session_expiry_seconds: u64,

    /// Tracks whether each config field was explicitly set or uses its default.
    /// Populated during construction; not a CLI argument.
    #[arg(skip)]
    value_sources: HashMap<String, ValueSource>,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        // Load .env file first
        dotenv().ok();
        // Parse CLI args and env vars, retaining ArgMatches so we can
        // inspect which values came from defaults vs explicit configuration
        let matches = Config::command().get_matches();
        let mut config =
            Config::from_arg_matches(&matches).expect("Failed to build Config from arg matches");

        // Capture the value source (Default, EnvVariable, CommandLine) for each field
        for field in CONFIG_FIELD_KEYS {
            if let Some(source) = matches.value_source(field) {
                config.value_sources.insert(field.to_string(), source);
            }
        }

        Self::warn_untracked_fields(&matches);

        config
    }

    /// Returns the names of any Clap args not listed in CONFIG_FIELD_KEYS.
    fn find_untracked_fields(matches: &clap::ArgMatches) -> Vec<String> {
        matches
            .ids()
            .filter_map(|id| {
                let name = id.as_str();
                if CONFIG_FIELD_KEYS.contains(&name) {
                    None
                } else {
                    Some(name.to_string())
                }
            })
            .collect()
    }

    /// Warns about any Clap args not listed in CONFIG_FIELD_KEYS so developers
    /// know they forgot to register a newly added config field.
    fn warn_untracked_fields(matches: &clap::ArgMatches) {
        for name in Self::find_untracked_fields(matches) {
            warn!(
                "Config field \"{}\" is not in CONFIG_FIELD_KEYS — \
                 add it to track its value source",
                name
            );
        }
    }

    /// Returns " (default)" if the field was not explicitly set via CLI or env var.
    fn source_suffix(&self, key: &str) -> &str {
        match self.value_sources.get(key) {
            Some(ValueSource::DefaultValue) => " (default)",
            _ => "",
        }
    }

    /// Logs all non-secret configuration values at DEBUG level.
    /// Secrets (API keys, auth keys, signing keys, database URL) are redacted.
    /// Appends " (default)" to any field not explicitly set via CLI or env var.
    pub fn log_non_secret_config(&self) {
        debug!("Configuration:");
        debug!(
            "  runtime_env: {}{}",
            self.runtime_env.display_value(),
            self.source_suffix("runtime_env")
        );
        debug!(
            "  api_version: {}{}",
            self.api_version.display_value(),
            self.source_suffix("api_version")
        );
        debug!(
            "  interface: {}{}",
            self.interface.display_value(),
            self.source_suffix("interface")
        );
        debug!(
            "  port: {}{}",
            self.port.display_value(),
            self.source_suffix("port")
        );
        debug!(
            "  log_level_filter: {}{}",
            self.log_level_filter.display_value(),
            self.source_suffix("log_level_filter")
        );
        debug!(
            "  allowed_origins: {}{}",
            self.allowed_origins.display_value(),
            self.source_suffix("allowed_origins")
        );
        debug!(
            "  db_max_connections: {}{}",
            self.db_max_connections.display_value(),
            self.source_suffix("db_max_connections")
        );
        debug!(
            "  db_min_connections: {}{}",
            self.db_min_connections.display_value(),
            self.source_suffix("db_min_connections")
        );
        debug!(
            "  db_connect_timeout_secs: {}{}",
            self.db_connect_timeout_secs.display_value(),
            self.source_suffix("db_connect_timeout_secs")
        );
        debug!(
            "  db_acquire_timeout_secs: {}{}",
            self.db_acquire_timeout_secs.display_value(),
            self.source_suffix("db_acquire_timeout_secs")
        );
        debug!(
            "  db_idle_timeout_secs: {}{}",
            self.db_idle_timeout_secs.display_value(),
            self.source_suffix("db_idle_timeout_secs")
        );
        debug!(
            "  db_max_lifetime_secs: {}{}",
            self.db_max_lifetime_secs.display_value(),
            self.source_suffix("db_max_lifetime_secs")
        );
        debug!(
            "  backend_session_expiry_seconds: {}{}",
            self.backend_session_expiry_seconds.display_value(),
            self.source_suffix("backend_session_expiry_seconds")
        );
        debug!(
            "  tiptap_app_id: {}{}",
            self.tiptap_app_id.display_value(),
            self.source_suffix("tiptap_app_id")
        );
        debug!(
            "  mailersend_base_url: {}{}",
            self.mailersend_base_url.display_value(),
            self.source_suffix("mailersend_base_url")
        );
        debug!(
            "  mailersend_api_key: {}{}",
            if self.mailersend_api_key.is_some() {
                "[set]"
            } else {
                "[not set]"
            },
            self.source_suffix("mailersend_api_key")
        );
        debug!(
            "  welcome_email_template_id: {}{}",
            self.welcome_email_template_id.display_value(),
            self.source_suffix("welcome_email_template_id")
        );
        debug!(
            "  session_scheduled_email_template_id: {}{}",
            self.session_scheduled_email_template_id.display_value(),
            self.source_suffix("session_scheduled_email_template_id")
        );
        debug!(
            "  action_assigned_email_template_id: {}{}",
            self.action_assigned_email_template_id.display_value(),
            self.source_suffix("action_assigned_email_template_id")
        );
        debug!(
            "  frontend_base_url: {}{}",
            self.frontend_base_url.display_value(),
            self.source_suffix("frontend_base_url")
        );
        debug!(
            "  session_scheduled_email_url_path: {}{}",
            self.session_scheduled_email_url_path.display_value(),
            self.source_suffix("session_scheduled_email_url_path")
        );
        debug!(
            "  action_assigned_email_url_path: {}{}",
            self.action_assigned_email_url_path.display_value(),
            self.source_suffix("action_assigned_email_url_path")
        );
    }

    pub fn api_version(&self) -> &str {
        self.api_version
            .as_ref()
            .expect("No API version string provided")
    }

    pub fn set_database_url(mut self, database_url: String) -> Self {
        self.database_url = Some(database_url);
        self
    }

    pub fn database_url(&self) -> &str {
        self.database_url
            .as_ref()
            .expect("No Database URL provided")
    }

    pub fn tiptap_url(&self) -> Option<String> {
        self.tiptap_url.clone()
    }

    pub fn tiptap_auth_key(&self) -> Option<String> {
        self.tiptap_auth_key.clone()
    }

    pub fn tiptap_jwt_signing_key(&self) -> Option<String> {
        self.tiptap_jwt_signing_key.clone()
    }

    pub fn tiptap_app_id(&self) -> Option<String> {
        self.tiptap_app_id.clone()
    }

    /// Returns the MailerSend API base URL.
    /// Falls back to the default if the configured value is empty.
    pub fn mailersend_base_url(&self) -> &str {
        if self.mailersend_base_url.is_empty() {
            DEFAULT_MAILERSEND_BASE_URL
        } else {
            &self.mailersend_base_url
        }
    }

    /// Returns the MailerSend API key, if configured.
    pub fn mailersend_api_key(&self) -> Option<String> {
        self.mailersend_api_key.clone()
    }

    /// Returns the MailerSend template ID for welcome emails, if configured.
    pub fn welcome_email_template_id(&self) -> Option<String> {
        self.welcome_email_template_id.clone()
    }

    /// Returns the MailerSend template ID for session-scheduled emails, if configured.
    pub fn session_scheduled_email_template_id(&self) -> Option<String> {
        self.session_scheduled_email_template_id.clone()
    }

    /// Returns the MailerSend template ID for action-assigned emails, if configured.
    pub fn action_assigned_email_template_id(&self) -> Option<String> {
        self.action_assigned_email_template_id.clone()
    }

    /// Returns the frontend application base URL used to construct links in emails.
    pub fn frontend_base_url(&self) -> Option<String> {
        self.frontend_base_url.clone()
    }

    /// Returns the URL path template for session-scheduled email links.
    /// Falls back to the default if the configured value is empty.
    pub fn session_scheduled_email_url_path(&self) -> &str {
        if self.session_scheduled_email_url_path.is_empty() {
            DEFAULT_SESSION_SCHEDULED_EMAIL_URL_PATH
        } else {
            &self.session_scheduled_email_url_path
        }
    }

    /// Returns the URL path template for action-assigned email links.
    /// Falls back to the default if the configured value is empty.
    pub fn action_assigned_email_url_path(&self) -> &str {
        if self.action_assigned_email_url_path.is_empty() {
            DEFAULT_ACTION_ASSIGNED_EMAIL_URL_PATH
        } else {
            &self.action_assigned_email_url_path
        }
    }

    pub fn runtime_env(&self) -> RustEnv {
        self.runtime_env.clone()
    }

    pub fn is_production(&self) -> bool {
        // This could check an environment variable, or a config field
        self.runtime_env() == RustEnv::Production
    }
}

impl ApiVersion {
    pub fn new(version_str: &'static str) -> Self {
        ApiVersion {
            version: Version::parse(version_str).unwrap_or(Version {
                major: 0,
                minor: 0,
                patch: 1,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }),
        }
    }

    pub fn default_version() -> &'static str {
        DEFAULT_API_VERSION
    }

    pub fn field_name() -> &'static str {
        X_VERSION
    }

    pub fn versions() -> APiVersionList {
        API_VERSIONS
    }
}

impl Default for ApiVersion {
    fn default() -> Self {
        ApiVersion {
            version: Version::parse(DEFAULT_API_VERSION).unwrap_or(Version {
                major: 0,
                minor: 0,
                patch: 1,
                pre: Prerelease::EMPTY,
                build: BuildMetadata::EMPTY,
            }),
        }
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a Config from simulated CLI args, capturing value sources
    /// the same way `Config::new()` does — but without parsing real process
    /// args or loading `.env`.
    fn config_from_args<I, T>(args: I) -> Config
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = Config::command()
            .try_get_matches_from(args)
            .expect("Failed to parse test args");
        let mut config =
            Config::from_arg_matches(&matches).expect("Failed to build Config from arg matches");

        for field in CONFIG_FIELD_KEYS {
            if let Some(source) = matches.value_source(field) {
                config.value_sources.insert(field.to_string(), source);
            }
        }

        config
    }

    #[test]
    fn unset_field_shows_default_value_and_suffix() {
        let config = config_from_args(["test_binary"]);

        assert_eq!(config.port.display_value(), "4000");
        assert_eq!(config.source_suffix("port"), " (default)");
    }

    #[test]
    fn explicitly_set_field_shows_actual_value_without_suffix() {
        let config = config_from_args(["test_binary", "--port", "8080"]);

        assert_eq!(config.port.display_value(), "8080");
        assert_eq!(config.source_suffix("port"), "");
    }

    #[test]
    fn all_config_fields_are_tracked() {
        let matches = Config::command()
            .try_get_matches_from(["test_binary"])
            .expect("Failed to parse test args");

        let untracked = Config::find_untracked_fields(&matches);
        assert!(
            untracked.is_empty(),
            "Config fields not in CONFIG_FIELD_KEYS: {:?}",
            untracked
        );
    }

    #[test]
    fn untracked_field_is_detected() {
        let matches = Config::command()
            .arg(clap::Arg::new("extra_test_field").long("extra-test-field"))
            .try_get_matches_from(["test_binary", "--extra-test-field", "value"])
            .expect("Failed to parse test args");

        let untracked = Config::find_untracked_fields(&matches);
        assert_eq!(untracked, vec!["extra_test_field"]);
    }
}
