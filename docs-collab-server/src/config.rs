//! CLI / env-driven configuration. Mirrors the `service` crate's clap-derive
//! idioms: `#[arg(long, env)]` fields, accessor methods, a `from_args` test
//! constructor that bypasses real process args.

use clap::{CommandFactory, FromArgMatches, Parser};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:1234";
const DEFAULT_SCHEMA: &str = "refactor_platform";
const DEFAULT_PERSIST_DEBOUNCE_MS: u64 = 500;
const DEFAULT_IDLE_EVICT_SECS: u64 = 300;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
pub struct Config {
    /// PostgreSQL connection string for the collaboration database.
    #[arg(
        long,
        env,
        default_value = "postgres://refactor:password@localhost:5432/refactor"
    )]
    database_url: String,

    /// Schema in which `collab_documents` lives. Created at startup if absent.
    #[arg(long, env, default_value = DEFAULT_SCHEMA)]
    database_schema: String,

    /// HS256 shared secret used to verify connection JWTs. MUST equal the
    /// signing key the application backend uses to mint them. Optional at
    /// parse time so `Default` works for tests; the server bootstrap checks
    /// for `Some` and refuses to start otherwise.
    #[arg(long, env)]
    jwt_signing_key: Option<String>,

    /// Shared secret required on the REST management endpoints. Compared
    /// verbatim to the `Authorization` header (no `Bearer ` prefix).
    #[arg(long, env)]
    management_auth_key: Option<String>,

    /// Host:port the server binds to.
    #[arg(long, env, default_value = DEFAULT_BIND_ADDR)]
    bind_addr: String,

    /// Coalesce update bursts for this many milliseconds before persisting.
    #[arg(long, env, default_value_t = DEFAULT_PERSIST_DEBOUNCE_MS)]
    persist_debounce_ms: u64,

    /// Evict a document this many seconds after the last connection leaves.
    #[arg(long, env, default_value_t = DEFAULT_IDLE_EVICT_SECS)]
    idle_evict_secs: u64,

    /// Maximum number of database connections in the pool.
    #[arg(long, env, default_value_t = 10)]
    db_max_connections: u32,

    /// Minimum number of idle database connections to maintain.
    #[arg(long, env, default_value_t = 1)]
    db_min_connections: u32,
}

impl Default for Config {
    /// Build a `Config` from clap's compiled-in defaults only. Does not read
    /// real CLI args, but `#[arg(env)]` fields still see process env vars.
    /// For tests that need a side-effect-free constructor.
    fn default() -> Self {
        Self::from_args(["docs-collab-server"])
    }
}

impl Config {
    /// Parse from real process CLI args and env vars.
    pub fn new() -> Self {
        Self::from_arg_matches(&Config::command().get_matches())
            .expect("Failed to build Config from arg matches")
    }

    /// Parse from an explicit arg list. Used by tests.
    pub fn from_args<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = Config::command()
            .try_get_matches_from(args)
            .expect("Failed to parse args");
        Self::from_arg_matches(&matches).expect("Failed to build Config from arg matches")
    }

    pub fn database_url(&self) -> &str {
        &self.database_url
    }

    pub fn database_schema(&self) -> &str {
        &self.database_schema
    }

    pub fn jwt_signing_key(&self) -> Option<&str> {
        self.jwt_signing_key.as_deref()
    }

    pub fn management_auth_key(&self) -> Option<&str> {
        self.management_auth_key.as_deref()
    }

    pub fn bind_addr(&self) -> &str {
        &self.bind_addr
    }

    pub fn persist_debounce_ms(&self) -> u64 {
        self.persist_debounce_ms
    }

    pub fn idle_evict_secs(&self) -> u64 {
        self.idle_evict_secs
    }

    pub fn db_max_connections(&self) -> u32 {
        self.db_max_connections
    }

    pub fn db_min_connections(&self) -> u32 {
        self.db_min_connections
    }
}
