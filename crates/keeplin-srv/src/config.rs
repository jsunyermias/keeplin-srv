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
    /// Reclaim the binary payloads of resources soft-deleted more than this many
    /// days ago (the metadata tombstone is always kept). `0` disables it. Mirrors
    /// the client's `resource_purge_days` (issue #24).
    pub resource_purge_days: u64,

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
    /// Maximum size in bytes of a resource binary upload
    /// (`PUT /api/resources/:id/data`). Larger bodies are rejected with `413`.
    pub max_upload_bytes: usize,
    /// Maximum size in bytes of a materialised note body (`GET /api/notes/:id`,
    /// `…/export`). A note whose joined lines exceed this is refused with `413`
    /// instead of being built in memory, bounding the read-path allocation
    /// (issue #44). `0` disables the cap. The collab line limits allow a note
    /// up to ~1 GB, so the default keeps a generous ceiling well above any real
    /// text note while refusing the pathological case.
    pub max_note_body_bytes: usize,

    // ── Per-user quotas (`0` disables each) ──────────────────────────────────
    /// Total bytes of resource binaries a single user may store. A blob upload
    /// that would push the user over this is rejected with `507`.
    pub max_user_storage_bytes: i64,
    /// Maximum number of live notes a single user may own. Creating one past
    /// this is rejected with `507`.
    pub max_notes_per_user: i64,

    // ── Access ────────────────────────────────────────────────────────────────
    /// Whether `POST /api/register` accepts new signups. Defaults to `true` for
    /// backward compatibility; set `false` on a private/single-tenant deployment
    /// so the open endpoint cannot be used to create accounts (issue #21). When
    /// `false`, registration returns `403`.
    pub registration_enabled: bool,
    /// Base64-encoded 32-byte key for at-rest encryption of note content and
    /// titles (issue keeplin#110), from `AT_REST_KEY`. `None` (unset) disables
    /// encryption and stores those fields as plaintext (backward compatible).
    pub at_rest_key: Option<String>,
    /// Where email delivery is delegated (issue #49): the server POSTs
    /// `{ kind, to, display_name, token, expires_at }` here and the operator's
    /// mail service composes and sends the message — keeplin never speaks SMTP.
    /// `None` disables the email flows (their endpoints answer `501`).
    pub mail_webhook_url: Option<String>,
    /// Optional bearer token sent in `Authorization` on webhook posts.
    pub mail_webhook_token: Option<String>,
    /// Lifetime of a verification/reset token, in seconds.
    pub email_token_ttl_secs: u64,
    /// When `true`, login refuses accounts that have not verified their email
    /// (`403`). Leave `false` unless the mail webhook is configured, or nobody
    /// can complete a login.
    pub email_verification_required: bool,
    /// Failed logins for one email before the account is temporarily locked
    /// (brute-force lockout; DB-backed so it holds across replicas). `0`
    /// disables the lockout.
    pub login_max_failures: i32,
    /// How long a lockout lasts, in seconds. Also the staleness window: a
    /// failure older than this restarts the counter instead of extending it.
    pub login_lockout_secs: u64,
    /// History visibility for shared notes/notebooks (issue #27). `false` (default,
    /// `HISTORY_VISIBILITY=creation`): everyone with read access sees the entity's full
    /// history from creation. `true` (`HISTORY_VISIBILITY=access`): a **collaborator** sees
    /// only versions from when they were granted access; the owner always sees everything.
    pub history_since_access: bool,
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// The historical dev placeholder. It is public in the source, so a token signed with it is
/// forgeable by anyone — it must never authenticate a real deployment.
const DEV_JWT_SECRET: &str = "dev-secret-change-in-production";

/// Minimum acceptable secret length (bytes). Short secrets are brute-forceable.
const MIN_JWT_SECRET_LEN: usize = 16;

/// Whether the operator explicitly opted into insecure local-dev behaviour.
fn dev_insecure() -> bool {
    std::env::var("KEEPLIN_DEV_INSECURE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// A secret that must not authenticate a real deployment: empty, the public dev
/// placeholder, or shorter than [`MIN_JWT_SECRET_LEN`].
fn is_weak_secret(s: &str) -> bool {
    s.trim().is_empty() || s == DEV_JWT_SECRET || s.len() < MIN_JWT_SECRET_LEN
}

/// Resolve `JWT_SECRET`, refusing to start on a missing, empty, too-short, or placeholder
/// value — otherwise the server would sign and verify every device token with a guessable
/// key, letting anyone forge a token for any user (issue #19). `KEEPLIN_DEV_INSECURE=1`
/// downgrades this to a loud warning for local development only.
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

impl Config {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_secrets_are_rejected() {
        assert!(is_weak_secret(""));
        assert!(is_weak_secret("   "));
        assert!(is_weak_secret(DEV_JWT_SECRET));
        assert!(is_weak_secret("short"));
        assert!(is_weak_secret(&"x".repeat(MIN_JWT_SECRET_LEN - 1)));
    }

    #[test]
    fn a_strong_secret_is_accepted() {
        assert!(!is_weak_secret(&"x".repeat(MIN_JWT_SECRET_LEN)));
        assert!(!is_weak_secret("a-genuinely-long-random-production-secret"));
    }
}
