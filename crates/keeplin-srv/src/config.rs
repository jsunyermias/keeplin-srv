#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    /// Device-token lifetime in days. Device tokens live in each daemon's
    /// config file and are presented on every (re)connect, so the default is
    /// long; rotate by logging in again.
    pub token_ttl_days: i64,
    /// Prune journal rows older than this many days once every device of the
    /// owning user has received them. `0` disables pruning.
    pub retention_days: u64,
    /// Compact line tombstones soft-deleted more than this many days ago
    /// (design §6.4). `0` disables the garbage collection.
    pub lines_gc_days: u64,

    // ── Operability ──────────────────────────────────────────────────────────
    /// Maximum PostgreSQL pool connections.
    pub db_max_connections: u32,
    /// Seconds to wait for a pooled connection (covers establishing a new one)
    /// before returning an error, instead of blocking a request forever.
    pub db_acquire_timeout_secs: u64,
    /// Close a pooled connection after this many idle seconds (reaps zombies).
    pub db_idle_timeout_secs: u64,
    /// Recycle a pooled connection after this many seconds of total life.
    pub db_max_lifetime_secs: u64,
    /// Per-client-IP request budget per minute (token bucket). `0` disables
    /// rate limiting. Behind a reverse proxy every request shares the proxy's
    /// IP, so rate-limit at the proxy instead and leave this at `0`.
    pub rate_limit_per_min: u32,
    /// Seconds to let in-flight work drain after a shutdown signal before the
    /// process force-exits (bounds long-lived WebSocket connections).
    pub shutdown_grace_secs: u64,
    /// Emit logs as JSON (one object per line) instead of the human-readable
    /// pretty format. Turn on in production for log aggregation.
    pub log_json: bool,
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: std::env::var("PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3000),
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            jwt_secret: std::env::var("JWT_SECRET")
                .unwrap_or_else(|_| "dev-secret-change-in-production".into()),
            token_ttl_days: std::env::var("TOKEN_TTL_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(365),
            retention_days: std::env::var("CHANGES_RETENTION_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            lines_gc_days: std::env::var("LINES_GC_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
            db_max_connections: env_parse("DB_MAX_CONNECTIONS", 10),
            db_acquire_timeout_secs: env_parse("DB_ACQUIRE_TIMEOUT_SECS", 10),
            db_idle_timeout_secs: env_parse("DB_IDLE_TIMEOUT_SECS", 600),
            db_max_lifetime_secs: env_parse("DB_MAX_LIFETIME_SECS", 1800),
            rate_limit_per_min: env_parse("RATE_LIMIT_PER_MIN", 0),
            shutdown_grace_secs: env_parse("SHUTDOWN_GRACE_SECS", 20),
            log_json: env_parse("LOG_JSON", false),
        }
    }
}
