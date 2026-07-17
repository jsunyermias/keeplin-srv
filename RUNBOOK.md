# keeplin-srv operator runbook

Operational procedures for running keeplin-srv in production. keeplin-srv is **stateless**: every
durable byte lives in **PostgreSQL** (accounts, devices, the relay journal, the collaborative note
model, the materialised notebooks/tags/resources, and the resource **binaries** in `resource_blobs`).
So operating keeplin-srv is, almost entirely, **operating its PostgreSQL** — back that up and you can
recreate the service anywhere.

The server applies its own schema migrations at startup (`sqlx::migrate!`), so a fresh binary against
an empty database is ready with no extra step.

## What to back up

| Item | Where | Notes |
|------|-------|-------|
| **PostgreSQL database** | your Postgres server | The whole product state. Includes resource binaries (`resource_blobs`, `BYTEA`), so backups grow with attachments — size accordingly. |
| `JWT_SECRET` | your secret store / env | Not in the DB. **Losing it invalidates every device token** (all devices must re-login). Back it up with your secrets, and keep it stable. |
| TLS certs / reverse-proxy config | your proxy host | keeplin-srv terminates no TLS itself; the proxy config is part of the deployment, not the DB. |

The server binary and config are reproducible (`Dockerfile`, `.env`), so they need no backup beyond
version control.

## Backup

### Simple: logical dump (pg_dump)

Good for small/medium instances and easy restores.

```bash
# Compressed custom-format dump (best for pg_restore; parallelisable).
pg_dump --format=custom --file=keeplin-$(date +%F).dump \
  "postgres://keeplin:PASSWORD@db-host:5432/keeplin"

# Verify the dump is readable before trusting it.
pg_restore --list keeplin-$(date +%F).dump >/dev/null && echo "dump OK"
```

Automate it (cron/systemd timer) and ship the file off-host (object storage). Keep several
generations; test a restore periodically (see below).

Note: `resource_blobs` can make dumps large. If attachments dominate, consider excluding blobs from
the frequent logical dump (`pg_dump --exclude-table=resource_blobs`) and relying on physical backups
(below) for full recovery — but only if you understand that a blob-less restore loses attachments.

### Robust: physical backups + PITR

For low RPO, use continuous archiving (WAL) so you can restore to any point in time. Either:

- a managed Postgres with automated backups + PITR (simplest — let the provider own it), or
- `pgBackRest` / `barman` / `pg_basebackup` + WAL archiving on a self-managed server.

This is the recommended posture for a multi-user deployment.

## Restore

### From a logical dump

```bash
# 1. Stop keeplin-srv (or take it out of the load balancer) so nothing writes mid-restore.
# 2. Create a fresh, empty database.
createdb -h db-host -U keeplin keeplin_restore

# 3. Restore (parallel jobs speed up large blob tables).
pg_restore --dbname="postgres://keeplin:PASSWORD@db-host:5432/keeplin_restore" \
  --jobs=4 --no-owner keeplin-YYYY-MM-DD.dump

# 4. Point DATABASE_URL at the restored database and start keeplin-srv.
#    Startup re-runs migrations (a no-op on an already-migrated database).
# 5. Smoke-test: GET /health -> 200, GET /ready -> 200 (DB reachable), then log in and list notes.
```

### From physical backup / PITR

Follow your tool's restore procedure (pgBackRest `restore`, provider console, etc.) to the target
time, then start keeplin-srv against the recovered cluster. The app needs no special handling — it
just connects and serves.

### After any restore

- Confirm `GET /health` returns `200`, `GET /ready` returns `200` (DB reachable), and (authenticated) `GET /api/metrics` shows sane row counts.
- Clients reconnect on their own (the daemon retries with backoff). Because the client database is a
  **cache**, devices re-hydrate notes/notebooks/tags/resources from the restored server; you do **not**
  restore anything on the clients.
- If you restored to a point **before** some client edits, those edits re-sync from the devices that
  still hold them (version vectors converge). Edits that existed **only** on the server and on no
  device are lost to the RPO of your backup — the usual backup trade-off.

## Routine maintenance

These run inside the server (the hourly `maintenance_loop`), tuned by env vars — no cron needed:

- **Relay journal pruning** (`CHANGES_RETENTION_DAYS`, default `0` = keep forever): rows delivered to
  every one of a user's devices are pruned after N days. Safe because the materialised tables — not
  the journal — are the source of truth. Set a value (e.g. `30`) once you have many devices and want
  bounded journal growth.
- **Line tombstone GC** (`LINES_GC_DAYS`, default `30`): reclaims deleted collaborative lines.

Neither affects backups beyond keeping the database smaller.

## Key rotation & re-encryption (`AT_REST_KEY`)

`AT_REST_KEY` encrypts `notes.title` and `lines.content` at rest (AES-256-GCM, values tagged
`enc:v1:`). Untagged rows are plaintext and stay readable, so the key can be enabled at any
time — but old rows remain plaintext until migrated.

### Enabling the key on an existing deployment (one-off re-encrypt pass)

```bash
# 1. Generate and set the key, restart the server (new writes are now encrypted).
openssl rand -base64 32          # -> AT_REST_KEY

# 2. Preview: how many pre-key plaintext rows are left?
keeplin-reencrypt --dry-run      # same env as the server (DATABASE_URL, AT_REST_KEY)

# 3. Migrate them. Safe against the live server; idempotent; resumable.
keeplin-reencrypt                # optionally --batch-size N (default 500)

# 4. Verify: a re-run finds nothing.
keeplin-reencrypt --dry-run      # -> 0 plaintext rows found
```

The pass processes bounded batches (one transaction each), logs progress per batch, skips any
row the live server rewrites concurrently (the server holds the same key, so that write is
already encrypted), and can be interrupted and re-run freely — finished batches stay done.

### Rotating the key

There is **no live key rotation**: the server reads exactly one key, and `keeplin-reencrypt`
encrypts to that same key — it cannot re-encrypt rows from an old key to a new one. The current
procedure is a maintenance-window swap:

1. Take a backup (as for any upgrade).
2. **With the old key still configured**, decrypt is possible; plan the swap as: stop writes
   (maintenance window), dump the affected columns decrypted (a small script using the old
   key), switch `AT_REST_KEY` to the new key, restart, rewrite the columns (they re-encrypt
   under the new key on write), run `keeplin-reencrypt` to catch anything left, verify, reopen.
3. Never delete the old key until every `enc:v1:` row verifiably decrypts under the new one.

Treat a suspected key compromise as a data breach first and a rotation second: rotating the
at-rest key does not un-leak whatever the compromised key already decrypted.

### Key backup — separate from database backups

Back `AT_REST_KEY` up in your **secret store, never next to the database dumps**. A backup
bundle containing both the dump and the key is plaintext for whoever steals it — the exact
threat at-rest encryption exists to stop. Conversely, **losing the key makes every `enc:v1:`
row permanently unreadable**; there is no recovery path.

## Capacity & quotas

- Resource binaries live in Postgres, so **database size tracks attachment volume**. Watch disk and
  set per-user limits: `MAX_USER_STORAGE_BYTES` (blob bytes/user) and `MAX_NOTES_PER_USER`
  (both `0` = unlimited). Over-quota writes get `507`.
- `MAX_UPLOAD_BYTES` caps a single upload.

## Upgrades

1. Back up (dump or ensure PITR is current).
2. Deploy the new binary/image. It applies any new migrations at startup; migrations are
   forward-only, so a mistake is corrected by a new migration, not a rollback.
3. Rolling restarts are safe: the server drains in-flight work on `SIGTERM` up to
   `SHUTDOWN_GRACE_SECS`, and clients reconnect automatically.

## Disaster-recovery drill

A backup that has never been restored is a hope, not a backup. `scripts/dr-drill.sh`
proves restorability end to end without touching the live database: it dumps the
source, restores into a throwaway database on the same server, verifies that the
row counts of every core table match, and drops the throwaway.

```bash
./scripts/dr-drill.sh "postgres://user:pass@host:5432/keeplin"
# … DR DRILL: PASS — dump restored and row counts match.
```

Run it on a schedule (monthly is a reasonable floor) and after any Postgres
upgrade. A non-zero exit is a paging-severity finding: your backups do not work.

## Monitoring & alerting

`GET /api/metrics` (auth required) serves JSON by default and the Prometheus
text format with `?format=prometheus` — point a scrape job at it with the
bearer token in the scrape config. On multi-replica deployments the
`keeplin_users/notes/lines/line_tombstones` gauges come from the shared
database (identical everywhere); `keeplin_collab_*` and
`keeplin_relay_live_users` are per-instance — scrape every replica and sum.

Minimum alert set:

| Alert | Condition | Why |
|-------|-----------|-----|
| Server down | `/health` non-200 | process died |
| Not ready | `/ready` non-200 for > 1 min | database unreachable — clients cannot work |
| Tombstone runaway | `keeplin_line_tombstones` growing without bound | `LINES_GC_DAYS` off or GC failing |
| Journal runaway | `changes` table size growing without bound | `CHANGES_RETENTION_DAYS` off, or a phantom device blocking pruning |
| Disk | Postgres volume > 80 % | attachments/journal growth |
| Login abuse | spike in `429` on `/api/login` (proxy logs) | credential-stuffing attempt hitting the lockout |
| Mail webhook failing | `mail webhook` errors in server logs | resets/verifications silently not delivered |

## Incident quick reference

| Symptom | First checks |
|---------|--------------|
| `/health` down | process up? |
| `/ready` 503 | `DATABASE_URL` reachable? Postgres accepting connections? pool exhausted? |
| Clients can't sync | proxy/TLS up? `wss://`/`https://` reachable? tokens still valid (was `JWT_SECRET` rotated)? |
| DB growing fast | attachment volume (`resource_blobs`) or an un-pruned journal — set `CHANGES_RETENTION_DAYS`, review quotas |
| Slow queries | pool exhausted (`DB_MAX_CONNECTIONS`, `DB_ACQUIRE_TIMEOUT_SECS`)? Postgres healthy / not swapping? |

## Related files

- `README.md` — configuration and quick start.
- `ARCHITECTURE.md` — the data model that lives in PostgreSQL.
- `.env.example` — every operational knob referenced above.

## Load / soak drill

`tests/soak.rs` (ignored in CI; run explicitly) drives N concurrent editors —
half against each of two bus-connected instances sharing one database — then
kills one instance mid-session:

```bash
DATABASE_URL=postgres://… cargo test --release --test soak -- --ignored --nocapture
# knobs: SOAK_EDITORS (default 8), SOAK_OPS (default 25)
```

It asserts the #45 guarantees: both instances settle on a byte-identical body,
and the survivor keeps accepting writes after a replica death. Reference run
(16 editors × 50 ops): 690 ops/s ingest, identical on both instances < 0.5 s
after the last send, replica-death survived. Note: causally-concurrent inserts
at the same position that lose the deterministic tiebreak are dropped by design
(the client re-diffs and self-heals); the soak reports that ratio.
