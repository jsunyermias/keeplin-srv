// md:Config
#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub jwt_secret: String,
    pub token_ttl_days: i64,
    pub retention_days: u64,
    pub lines_gc_days: u64,
    pub resource_purge_days: u64,
    pub db_max_connections: u32,
    pub db_acquire_timeout_secs: u64,
    pub db_idle_timeout_secs: u64,
    pub db_max_lifetime_secs: u64,
    pub rate_limit_per_min: u32,
    pub shutdown_grace_secs: u64,
    pub log_json: bool,
    pub max_upload_bytes: usize,
    pub max_note_body_bytes: usize,
    pub max_user_storage_bytes: i64,
    pub max_notes_per_user: i64,
    pub registration_enabled: bool,
    pub at_rest_key: Option<String>,
    pub mail_webhook_url: Option<String>,
    pub mail_webhook_token: Option<String>,
    pub email_token_ttl_secs: u64,
    pub email_verification_required: bool,
    pub login_max_failures: i32,
    pub login_lockout_secs: u64,
    pub history_since_access: bool,
}

// md:fn env_parse
fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

// md:JWT secret constants
const DEV_JWT_SECRET: &str = "dev-secret-change-in-production";
const MIN_JWT_SECRET_LEN: usize = 16;

// md:fn dev_insecure
fn dev_insecure() -> bool {
    std::env::var("KEEPLIN_DEV_INSECURE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

// md:fn is_weak_secret
fn is_weak_secret(s: &str) -> bool {
    s.trim().is_empty() || s == DEV_JWT_SECRET || s.len() < MIN_JWT_SECRET_LEN
}

// md:fn resolve_jwt_secret
fn resolve_jwt_secret() -> String {
    let raw = std::env::var("JWT_SECRET").ok();
    match raw {
        Some(s) if !is_weak_secret(&s) => s,
        other => {
            if dev_insecure() {
                tracing::warn!(
                    "KEEPLIN_DEV_INSECURE=1: using an insecure JWT_SECRET — device tokens are \
                     forgeable. NEVER do this in production."
                );
                other
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| DEV_JWT_SECRET.into())
            } else {
                panic!(
                    "JWT_SECRET must be set to a strong random secret of at least \
                     {MIN_JWT_SECRET_LEN} characters (not empty and not the dev placeholder). \
                     Without it, device tokens can be forged. Set JWT_SECRET, or set \
                     KEEPLIN_DEV_INSECURE=1 for local development only."
                );
            }
        }
    }
}

// md:impl Config
impl Config {
    // md:impl Config > fn from_env
    pub fn from_env() -> Self {
        Self {
            port: std::env::var("PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3000),
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            jwt_secret: resolve_jwt_secret(),
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
            resource_purge_days: env_parse("RESOURCE_PURGE_DAYS", 0),
            db_max_connections: env_parse("DB_MAX_CONNECTIONS", 10),
            db_acquire_timeout_secs: env_parse("DB_ACQUIRE_TIMEOUT_SECS", 10),
            db_idle_timeout_secs: env_parse("DB_IDLE_TIMEOUT_SECS", 600),
            db_max_lifetime_secs: env_parse("DB_MAX_LIFETIME_SECS", 1800),
            rate_limit_per_min: env_parse("RATE_LIMIT_PER_MIN", 0),
            shutdown_grace_secs: env_parse("SHUTDOWN_GRACE_SECS", 20),
            log_json: env_parse("LOG_JSON", false),
            max_upload_bytes: env_parse("MAX_UPLOAD_BYTES", 100 * 1024 * 1024),
            max_note_body_bytes: env_parse("MAX_NOTE_BODY_BYTES", 25 * 1024 * 1024),
            max_user_storage_bytes: env_parse("MAX_USER_STORAGE_BYTES", 0),
            max_notes_per_user: env_parse("MAX_NOTES_PER_USER", 0),
            at_rest_key: std::env::var("AT_REST_KEY")
                .ok()
                .filter(|k| !k.trim().is_empty()),
            mail_webhook_url: std::env::var("MAIL_WEBHOOK_URL")
                .ok()
                .filter(|u| !u.trim().is_empty()),
            mail_webhook_token: std::env::var("MAIL_WEBHOOK_TOKEN")
                .ok()
                .filter(|t| !t.trim().is_empty()),
            email_token_ttl_secs: env_parse("EMAIL_TOKEN_TTL_SECS", 3600),
            email_verification_required: env_parse("EMAIL_VERIFICATION_REQUIRED", false),
            login_max_failures: env_parse("LOGIN_MAX_FAILURES", 10),
            login_lockout_secs: env_parse("LOGIN_LOCKOUT_SECS", 300),
            registration_enabled: env_parse("REGISTRATION_ENABLED", true),
            history_since_access: std::env::var("HISTORY_VISIBILITY")
                .map(|v| v.eq_ignore_ascii_case("access"))
                .unwrap_or(false),
        }
    }
}

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn weak_secrets_are_rejected
    #[test]
    fn weak_secrets_are_rejected() {
        assert!(is_weak_secret(""));
        assert!(is_weak_secret("   "));
        assert!(is_weak_secret(DEV_JWT_SECRET));
        assert!(is_weak_secret("short"));
        assert!(is_weak_secret(&"x".repeat(MIN_JWT_SECRET_LEN - 1)));
    }

    // md:mod tests > fn a_strong_secret_is_accepted
    #[test]
    fn a_strong_secret_is_accepted() {
        assert!(!is_weak_secret(&"x".repeat(MIN_JWT_SECRET_LEN)));
        assert!(!is_weak_secret("a-genuinely-long-random-production-secret"));
    }
}
