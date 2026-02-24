use clap::builder::TypedValueParser as _;
use clap::Parser;
use dotenvy::dotenv;
use log::LevelFilter;
use semver::{BuildMetadata, Prerelease, Version};
use serde::Deserialize;
use std::fmt;
use std::str::FromStr;
use utoipa::IntoParams;

type APiVersionList = [&'static str; 1];

const DEFAULT_API_VERSION: &str = "1.0.0-beta1";
// Expand this array to include all valid API versions. Versions that have been
// completely removed should be removed from this list - they're no longer valid.
const API_VERSIONS: APiVersionList = [DEFAULT_API_VERSION];

static X_VERSION: &str = "x-version";

/// Default MailerSend API base URL used when `MAILERSEND_BASE_URL` is not set.
pub const DEFAULT_MAILERSEND_BASE_URL: &str = "https://api.mailersend.com/v1";

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
        // Then parse the command line parameters and flags
        Config::parse()
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
    pub fn mailersend_base_url(&self) -> &str {
        &self.mailersend_base_url
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
    pub fn session_scheduled_email_url_path(&self) -> &str {
        &self.session_scheduled_email_url_path
    }

    /// Returns the URL path template for action-assigned email links.
    pub fn action_assigned_email_url_path(&self) -> &str {
        &self.action_assigned_email_url_path
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
