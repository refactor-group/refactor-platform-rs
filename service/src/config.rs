use clap::builder::TypedValueParser as _;
use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches, Parser};
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

/// Default URL path for magic link setup page.
const DEFAULT_MAGIC_LINK_EMAIL_URL_PATH: &str = "/setup/{token}";

/// Default expiry duration for magic link tokens (72 hours in seconds).
const DEFAULT_MAGIC_LINK_EXPIRY_SECONDS: u64 = 259200;

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
    "magic_link_email_url_path",
    "magic_link_expiry_seconds",
    "interface",
    "port",
    "log_level_filter",
    "runtime_env",
    "backend_session_expiry_seconds",
    "oauth_success_redirect_uri",
    "google_oauth_auth_url",
    "google_oauth_token_url",
    "google_userinfo_url",
    "google_meet_api_url",
    "zoom_api_url",
    "recall_ai_api_key",
    "recall_ai_region",
    "recall_ai_webhook_secret",
    "webhook_base_url",
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
            None => "[unset]".to_string(),
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
    /// URL path template for magic link setup page.
    /// Use `{token}` as a placeholder for the magic link token.
    #[arg(long, env, default_value = DEFAULT_MAGIC_LINK_EMAIL_URL_PATH)]
    magic_link_email_url_path: String,
    /// Expiry duration in seconds for magic link tokens (default: 72 hours).
    #[arg(long, env, default_value_t = DEFAULT_MAGIC_LINK_EXPIRY_SECONDS)]
    magic_link_expiry_seconds: u64,

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

    /// 32-byte AES encryption key for encrypting sensitive API keys in database (hex-encoded)
    #[arg(long, env)]
    encryption_key: Option<String>,

    /// Google OAuth client ID
    #[arg(long, env)]
    google_client_id: Option<String>,

    /// Google OAuth client secret
    #[arg(long, env)]
    google_client_secret: Option<String>,

    /// Google OAuth redirect URI (callback from Google to backend)
    #[arg(long, env)]
    google_redirect_uri: Option<String>,

    /// URL to redirect to after successful Provider OAuth (frontend settings page)
    #[arg(long, env, default_value = "http://localhost:3000/settings")]
    oauth_success_redirect_uri: String,

    /// Google OAuth authorization URL
    #[arg(
        long,
        env,
        default_value = "https://accounts.google.com/o/oauth2/v2/auth"
    )]
    google_oauth_auth_url: String,

    /// Google OAuth token URL
    #[arg(long, env, default_value = "https://oauth2.googleapis.com/token")]
    google_oauth_token_url: String,

    /// Google user info URL
    #[arg(
        long,
        env,
        default_value = "https://www.googleapis.com/oauth2/v2/userinfo"
    )]
    google_userinfo_url: String,

    /// Google Meet API base URL
    #[arg(long, env, default_value = "https://meet.googleapis.com/v2")]
    google_meet_api_url: String,

    /// Zoom OAuth client ID
    #[arg(long, env)]
    zoom_client_id: Option<String>,

    /// Zoom OAuth client secret
    #[arg(long, env)]
    zoom_client_secret: Option<String>,

    /// Zoom OAuth redirect URI (callback from Zoom to backend)
    #[arg(long, env)]
    zoom_redirect_uri: Option<String>,

    /// Zoom meeting API base URL
    #[arg(long, env, default_value = "https://api.zoom.us/v2")]
    zoom_api_url: String,

    /// Recall.ai API key (system-level; used for bot creation and async transcription)
    #[arg(long, env)]
    recall_ai_api_key: Option<String>,

    /// Recall.ai region (us or eu)
    #[arg(long, env, default_value = "use-east-1")]
    recall_ai_region: String,

    /// Recall.ai webhook signing secret (Svix `whsec_...` format)
    #[arg(long, env)]
    recall_ai_webhook_secret: Option<String>,

    /// Public base URL used to construct webhook callback URLs (e.g. https://app.refactorcoach.com)
    #[arg(long, env)]
    webhook_base_url: Option<String>,

    /// Tracks whether each config field was explicitly set or uses its default.
    /// Populated during construction; not a CLI argument.
    #[arg(skip)]
    value_sources: HashMap<String, ValueSource>,
}

impl Default for Config {
    /// Returns a `Config` with clap's compiled-in defaults. Does not read
    /// real CLI args or `.env` files. Shell env vars are still read via
    /// clap's `#[arg(env)]` attributes.
    ///
    /// Side effect: may call `std::env::remove_var` on any `#[arg(env)]`
    /// var whose current value is empty or whitespace-only (see
    /// `sanitize_empty_env`).
    fn default() -> Self {
        Self::from_args(["refactor-platform-rs"])
    }
}

impl Config {
    /// Parse config from real process CLI args and env vars.
    ///
    /// **Important:** Call [`service::load_env_file`] before this if you want
    /// `.env` file values to be available. This method intentionally does not
    /// load `.env` itself so that test code can use [`Config::default`] or
    /// [`Config::from_args`] without side effects.
    pub fn new() -> Self {
        Self::sanitize_empty_env(&Config::command());
        let matches = Config::command().get_matches();
        let mut config =
            Config::from_arg_matches(&matches).expect("Failed to build Config from arg matches");

        config.capture_value_sources(&matches);
        Self::warn_untracked_fields(&matches);

        config
    }

    /// Parse config from an explicit argument list. Does not read real CLI
    /// args. Clap's `#[arg(env)]` fields still read from process env vars,
    /// but `.env` is not loaded.
    ///
    /// Primarily useful for tests that need a `Config` with specific values.
    pub fn from_args<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::sanitize_empty_env(&Config::command());
        let matches = Config::command()
            .try_get_matches_from(args)
            .expect("Failed to parse args");
        let mut config =
            Config::from_arg_matches(&matches).expect("Failed to build Config from arg matches");

        config.capture_value_sources(&matches);

        config
    }

    /// Unset any process env var that clap is configured to read (via
    /// `#[arg(env)]`) whose value is empty or whitespace-only.
    ///
    /// Why: Docker Compose expands `KEY: ${KEY}` to an empty string when the
    /// host var is unset, which clap treats as a set-but-unparseable value —
    /// crashing primitive-typed fields like `u32`/`u64` on startup. Treating
    /// empty env as unset here lets clap fall back to `default_value_t`
    /// uniformly for every field, without hardcoding which ones need it.
    /// Self-maintaining: new `#[arg(env)]` fields get this behavior for free.
    ///
    /// The caller must pass a *throwaway* `Command` and build a fresh one for
    /// actual parsing — clap 4 snapshots env values when each `Arg` is
    /// constructed during `augment_args`, not when `get_matches()` is called,
    /// so the `Command` used for parsing must be built *after* this sanitize
    /// runs for the cleanup to take effect.
    fn sanitize_empty_env(cmd: &clap::Command) {
        for arg in cmd.get_arguments() {
            let Some(env_name) = arg.get_env() else {
                continue;
            };
            let Ok(val) = std::env::var(env_name) else {
                continue;
            };
            if val.trim().is_empty() {
                std::env::remove_var(env_name);
            }
        }
    }

    /// Records the value source (Default, EnvVariable, CommandLine) for every
    /// field listed in CONFIG_FIELD_KEYS. Called during construction so that
    /// `source_suffix()` can annotate log output later.
    fn capture_value_sources(&mut self, matches: &clap::ArgMatches) {
        for field in CONFIG_FIELD_KEYS {
            if let Some(source) = matches.value_source(field) {
                self.value_sources.insert(field.to_string(), source);
            }
        }
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

    /// Emits a single config field at DEBUG level with its value and source suffix.
    fn debug_field(&self, name: &str, value: &dyn ConfigDisplay) {
        debug!(
            "  {}: {}{}",
            name,
            value.display_value(),
            self.source_suffix(name)
        );
    }

    /// Logs all non-secret configuration values at DEBUG level.
    /// Secrets (API keys, auth keys, signing keys, database URL) are redacted.
    /// Appends " (default)" to any field not explicitly set via CLI or env var.
    pub fn log_non_secret_config(&self) {
        debug!("Configuration:");
        self.debug_field("runtime_env", &self.runtime_env);
        self.debug_field("api_version", &self.api_version);
        self.debug_field("interface", &self.interface);
        self.debug_field("port", &self.port);
        self.debug_field("log_level_filter", &self.log_level_filter);
        self.debug_field("allowed_origins", &self.allowed_origins);
        self.debug_field("db_max_connections", &self.db_max_connections);
        self.debug_field("db_min_connections", &self.db_min_connections);
        self.debug_field("db_connect_timeout_secs", &self.db_connect_timeout_secs);
        self.debug_field("db_acquire_timeout_secs", &self.db_acquire_timeout_secs);
        self.debug_field("db_idle_timeout_secs", &self.db_idle_timeout_secs);
        self.debug_field("db_max_lifetime_secs", &self.db_max_lifetime_secs);
        self.debug_field(
            "backend_session_expiry_seconds",
            &self.backend_session_expiry_seconds,
        );
        self.debug_field("tiptap_app_id", &self.tiptap_app_id);
        self.debug_field("mailersend_base_url", &self.mailersend_base_url);
        self.debug_field("welcome_email_template_id", &self.welcome_email_template_id);
        self.debug_field(
            "session_scheduled_email_template_id",
            &self.session_scheduled_email_template_id,
        );
        self.debug_field(
            "action_assigned_email_template_id",
            &self.action_assigned_email_template_id,
        );
        self.debug_field("frontend_base_url", &self.frontend_base_url);
        self.debug_field(
            "session_scheduled_email_url_path",
            &self.session_scheduled_email_url_path,
        );
        self.debug_field(
            "action_assigned_email_url_path",
            &self.action_assigned_email_url_path,
        );
        self.debug_field("magic_link_email_url_path", &self.magic_link_email_url_path);
        self.debug_field("magic_link_expiry_seconds", &self.magic_link_expiry_seconds);
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

    /// Returns the URL path template for magic link setup page.
    /// Falls back to the default if the configured value is empty.
    pub fn magic_link_email_url_path(&self) -> &str {
        if self.magic_link_email_url_path.is_empty() {
            DEFAULT_MAGIC_LINK_EMAIL_URL_PATH
        } else {
            &self.magic_link_email_url_path
        }
    }

    /// Returns the expiry duration in seconds for magic link tokens.
    pub fn magic_link_expiry_seconds(&self) -> u64 {
        self.magic_link_expiry_seconds
    }

    pub fn runtime_env(&self) -> RustEnv {
        self.runtime_env.clone()
    }

    pub fn is_production(&self) -> bool {
        // This could check an environment variable, or a config field
        self.runtime_env() == RustEnv::Production
    }

    // AI Meeting Integration accessors

    pub fn encryption_key(&self) -> Option<String> {
        self.encryption_key.clone()
    }

    pub fn google_client_id(&self) -> Option<String> {
        self.google_client_id.clone()
    }

    pub fn google_client_secret(&self) -> Option<String> {
        self.google_client_secret.clone()
    }

    pub fn google_redirect_uri(&self) -> Option<String> {
        self.google_redirect_uri.clone()
    }

    pub fn oauth_success_redirect_uri(&self) -> &str {
        &self.oauth_success_redirect_uri
    }

    pub fn google_oauth_auth_url(&self) -> &str {
        &self.google_oauth_auth_url
    }

    pub fn google_oauth_token_url(&self) -> &str {
        &self.google_oauth_token_url
    }

    pub fn google_userinfo_url(&self) -> &str {
        &self.google_userinfo_url
    }

    pub fn google_meet_api_url(&self) -> &str {
        &self.google_meet_api_url
    }

    pub fn zoom_client_id(&self) -> Option<String> {
        self.zoom_client_id.clone()
    }

    pub fn zoom_client_secret(&self) -> Option<String> {
        self.zoom_client_secret.clone()
    }

    pub fn zoom_redirect_uri(&self) -> Option<String> {
        self.zoom_redirect_uri.clone()
    }

    pub fn zoom_api_url(&self) -> &str {
        &self.zoom_api_url
    }

    // Recall.ai / Meeting AI accessors

    pub fn recall_ai_api_key(&self) -> Option<String> {
        self.recall_ai_api_key.clone()
    }

    pub fn recall_ai_region(&self) -> &str {
        &self.recall_ai_region
    }

    pub fn recall_ai_webhook_secret(&self) -> Option<String> {
        self.recall_ai_webhook_secret.clone()
    }

    pub fn webhook_base_url(&self) -> Option<String> {
        self.webhook_base_url.clone()
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
    use serial_test::serial;
    use std::fs;

    #[test]
    fn unset_field_shows_default_value_and_suffix() {
        let config = Config::from_args(["test_binary"]);

        assert_eq!(config.port.display_value(), "4000");
        assert_eq!(config.source_suffix("port"), " (default)");
    }

    #[test]
    fn explicitly_set_field_shows_actual_value_without_suffix() {
        let config = Config::from_args(["test_binary", "--port", "8080"]);

        assert_eq!(config.port.display_value(), "8080");
        assert_eq!(config.source_suffix("port"), "");
    }

    // Marked `#[serial]` because iterating `matches.ids()` while another
    // thread is mutating env vars or building its own augmented Command
    // races on clap's internal state — the command name ("Config") leaks
    // into the id iterator. Serializing avoids the race.
    #[test]
    #[serial]
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
    #[serial]
    fn untracked_field_is_detected() {
        let matches = Config::command()
            .arg(clap::Arg::new("extra_test_field").long("extra-test-field"))
            .try_get_matches_from(["test_binary", "--extra-test-field", "value"])
            .expect("Failed to parse test args");

        let untracked = Config::find_untracked_fields(&matches);
        assert_eq!(untracked, vec!["extra_test_field"]);
    }

    /// RAII guard that restores a process env var to its prior state on drop.
    /// Use with `#[serial]` so concurrent tests do not race on the env table.
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        fn unset(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    #[serial]
    fn env_var_populates_field_when_no_flag() {
        let _guard = EnvGuard::set("MAILERSEND_API_KEY", "from_env");
        let config = Config::from_args(["test_binary"]);
        assert_eq!(config.mailersend_api_key(), Some("from_env".to_string()));
    }

    #[test]
    #[serial]
    fn cli_flag_overrides_env_var() {
        let _guard = EnvGuard::set("MAILERSEND_API_KEY", "from_env");
        let config = Config::from_args(["test_binary", "--mailersend-api-key", "from_cli"]);
        assert_eq!(config.mailersend_api_key(), Some("from_cli".to_string()));
    }

    #[test]
    #[serial]
    fn value_source_records_env_for_env_sourced_field() {
        let _guard = EnvGuard::set("MAILERSEND_API_KEY", "from_env");
        let config = Config::from_args(["test_binary"]);
        assert_eq!(
            config.value_sources.get("mailersend_api_key"),
            Some(&ValueSource::EnvVariable),
        );
    }

    /// Regression guard for the parent test runner in `src/main.rs`. The
    /// runner calls `Config::default()` and then spawns child cargo
    /// processes that inherit its env. If `Config::default()` ever started
    /// loading `.env` (e.g. via a misplaced `dotenvy::dotenv()`), child
    /// processes would inherit those values and tests like
    /// `domain::emails::tests::test_send_*_missing_template_id` would
    /// silently start failing again.
    #[test]
    #[serial]
    fn default_constructor_does_not_load_env_file() {
        let _key_guard = EnvGuard::unset("MAILERSEND_API_KEY");

        let temp_dir = std::env::temp_dir().join(format!(
            "refactor-config-default-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(
            temp_dir.join(".env"),
            "MAILERSEND_API_KEY=should_not_be_loaded\n",
        )
        .unwrap();

        let original_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let config = Config::default();

        // Restore cwd and clean up before any assertion that might panic.
        std::env::set_current_dir(&original_cwd).unwrap();
        fs::remove_dir_all(&temp_dir).ok();

        assert!(
            config.mailersend_api_key().is_none(),
            "Config::default() must not load .env (got {:?})",
            config.mailersend_api_key()
        );
        assert!(
            std::env::var("MAILERSEND_API_KEY").is_err(),
            "Config::default() must not write .env values into the process env",
        );
    }
}
