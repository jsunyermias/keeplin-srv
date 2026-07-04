-- Accounts. One row per registered user; the relay partitions all sync
-- traffic by user, so a user's devices only ever see that user's changes.
CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One row per device login. Each keeplin-daemon instance must log in once and
-- use its own token: the device id inside the token is the relay's identity for
-- the connection (echo suppression + delivery cursor), so sharing a token
-- between two machines would make them invisible to each other.
CREATE TABLE IF NOT EXISTS user_devices (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_user_devices_user ON user_devices(user_id);

-- The change journal: every `Change` received from any device, in arrival
-- order. The payload is stored as opaque JSON — the relay never interprets
-- keeplin-core's `Change` enum, it only forwards it, so client model changes
-- never require a server migration.
--
-- (batch_id, batch_index) dedupes client retries: `send_changes` may re-send
-- the same batch after a reconnect, and the second insert becomes a no-op.
CREATE TABLE IF NOT EXISTS changes (
    seq BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    origin_device_id UUID NOT NULL,
    batch_id UUID NOT NULL,
    batch_index INTEGER NOT NULL,
    sync_device_id TEXT NOT NULL DEFAULT '',
    payload JSONB NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (batch_id, batch_index)
);

CREATE INDEX IF NOT EXISTS idx_changes_user_seq ON changes(user_id, seq);

-- Per-device delivery watermark: every change with seq <= last_seq has either
-- been delivered to this device or originated from it. Devices with no row
-- start at 0 and receive the full journal on first connect.
CREATE TABLE IF NOT EXISTS device_cursors (
    device_id UUID PRIMARY KEY REFERENCES user_devices(id) ON DELETE CASCADE,
    last_seq BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
