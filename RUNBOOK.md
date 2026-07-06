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
# 5. Smoke-test: GET /health -> 200, then log in and list notes.
```

### From physical backup / PITR

Follow your tool's restore procedure (pgBackRest `restore`, provider console, etc.) to the target
time, then start keeplin-srv against the recovered cluster. The app needs no special handling — it
just connects and serves.

### After any restore

- Confirm `GET /health` returns `200` and `GET /api/metrics` shows sane row counts.
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

## Incident quick reference

| Symptom | First checks |
|---------|--------------|
| `/health` down | process up? `DATABASE_URL` reachable? Postgres accepting connections? |
| Clients can't sync | proxy/TLS up? `wss://`/`https://` reachable? tokens still valid (was `JWT_SECRET` rotated)? |
| DB growing fast | attachment volume (`resource_blobs`) or an un-pruned journal — set `CHANGES_RETENTION_DAYS`, review quotas |
| Slow queries | pool exhausted (`DB_MAX_CONNECTIONS`, `DB_ACQUIRE_TIMEOUT_SECS`)? Postgres healthy / not swapping? |

## Related files

- `README.md` — configuration and quick start.
- `ARCHITECTURE.md` — the data model that lives in PostgreSQL.
- `.env.example` — every operational knob referenced above.
