# `main.rs` — keeplin-srv entry point

## Purpose

Builds the process and starts the server: initialise logging, open a bounded PostgreSQL pool,
run migrations, spawn the background maintenance loop, and serve the axum router with graceful
shutdown. All request-handling logic lives in the library (`lib.rs` and its modules); this
file is the wiring.

## Startup / wiring

```
1. dotenvy::dotenv()                     — load .env if present
2. Config::from_env()                    — read all settings (see config.md)
3. tracing_subscriber                    — JSON logs if LOG_JSON, else pretty
4. PgPoolOptions                         — max_connections + acquire/idle/max-lifetime timeouts
5. sqlx::migrate!("../../migrations")    — apply pending schema migrations
6. AppState::new(config, pool)           — construct shared state (incl. rate limiter)
7. spawn maintenance_loop                — hourly journal prune + line-tombstone GC (if enabled)
8. axum::serve(...).into_make_service_with_connect_info::<SocketAddr>()
       .with_graceful_shutdown(shutdown_signal(grace))
```

## Graceful shutdown

`shutdown_signal(grace)` resolves on `SIGTERM` (containers/systemd) or `Ctrl-C`. axum's
graceful shutdown then drains in-flight REST requests. Because collaborative WebSocket
connections are long-lived and would otherwise keep the process alive forever, the same
signal arms a **watchdog** that `std::process::exit(0)` after `SHUTDOWN_GRACE_SECS`. This
bounds shutdown so the server is safe under systemd/Kubernetes rolling restarts.

## Maintenance loop

Runs once an hour when either knob is on:

- `CHANGES_RETENTION_DAYS > 0` → `store.prune_delivered_changes` deletes relay-journal rows
  already delivered to every device of the owning user.
- `LINES_GC_DAYS > 0` → `store.gc_line_tombstones` compacts lines soft-deleted long ago
  (design §6.4).

## Design notes

- `into_make_service_with_connect_info::<SocketAddr>()` is required so the rate-limit
  middleware can key on the peer IP; the integration tests do the same when they spawn the
  router.
- The pool caps and timeouts turn an exhausted pool into a fast per-request error instead of
  an indefinite hang, and reap idle/old connections so zombies do not accumulate.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `main()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `maintenance_loop()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `run_retention()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `shutdown_signal()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×1; e.g. `router()`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×2; e.g. `AppState`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Startup must fail fast on invalid config: missing `DATABASE_URL`, weak `JWT_SECRET` (without `KEEPLIN_DEV_INSECURE=1`), malformed `AT_REST_KEY`.
- Migrations run at startup via `sqlx::migrate!` and are forward-only; the binary must remain runnable against an already-migrated database (no-op).
- Graceful shutdown drains in-flight work but force-exits after `SHUTDOWN_GRACE_SECS` (long-lived WebSockets would otherwise pin the process forever).
- The maintenance loop owns presence heartbeat/sweep and the retention passes; disabling retention knobs must not disable presence upkeep.

## Related files

- `config.md` — every environment knob this file reads.
- `store.md` — the maintenance queries the loop calls.
- `http.md` — the router being served.
