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
                .unwrap_or_else(|_| "dev-secret-cambia-en-produccion".into()),
            token_ttl_days: std::env::var("TOKEN_TTL_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(365),
            retention_days: std::env::var("CHANGES_RETENTION_DAYS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        }
    }
}
