# keeplin-srv

El servidor multiusuario de [Keeplin](https://github.com/jsunyermias/keeplin) con
**edición colaborativa en tiempo real por líneas**: varios usuarios editan la misma
nota simultáneamente, estilo Google Docs pero sobre Markdown, manteniendo los mismos
conceptos que keeplin-core — `VersionVector`, `last_writer`, `updated_at` y
tombstones con soft-delete. Sin bloqueos: la resolución es siempre por version
vector (`note_log::resolve`), nunca por lock.

Escrito en Rust (axum + PostgreSQL).

## Modelo

- **La unidad de concurrencia es la línea.** Cada línea es una entidad versionada
  independiente que se crea, edita, borra (tombstone) y resuelve por sí sola.
- **El orden de líneas es otra entidad versionada** con su propio `vv`,
  `updated_at` y `last_writer`. Contiene todos los `line_id`, incluidos los
  borrados, hasta garbage collection.
- **El servidor es broker y fuente de verdad duradera**: valida cada operación,
  la resuelve contra el estado actual, la persiste y la reenvía a los demás
  suscriptores de la nota. Los clientes son stateful y reconstruyen desde el
  snapshot al (re)conectar — no hay log de operaciones infinito.
- El `body` de una nota no se almacena: se materializa concatenando las líneas
  vivas con `\n` para las lecturas REST no colaborativas.

## Protocolo colaborativo (`GET /api/ws?token=<jwt>`)

Mensajes JSON con campo `type`:

- Cliente → servidor: `Join { note_id }`, `Leave { note_id }`,
  `Op { note_id, ops: [LineOp…] }`, `Cursor { note_id, cursor }`, `Ack { server_seq }`.
- Servidor → cliente: `Welcome { note_id, snapshot }` (orden versionado + todas las
  líneas), `Op { server_seq, note_id, user_id, ops }`, `Presence { note_id, users }`,
  `Error { code, message }`.

`LineOp` (`op`): `Insert { after_line_id, line_id, content, vv, last_writer, updated_at }`,
`Update`, `Delete` (tombstone) y `Move { line_ids, after_line_id, … }`. Cada operación
lleva su propio `vv`; el servidor exige que el `last_writer` sea el usuario
autenticado y que el vector avance el componente del escritor.

**Resolución** (§5 del diseño): por línea, `resolve(local, incoming)` — la operación
dominada se ignora; las concurrentes se deciden por el tiebreak determinista
`(updated_at, last_writer)`, idéntico en todas las réplicas. `Insert`/`Move` se
resuelven contra la entidad de orden.

**Límites**: 10 000 caracteres por línea, 100 000 líneas por nota, 1 MB por mensaje.

## API REST

- `GET /health`
- `POST /api/register` — `{ email, password, display_name? }`
- `POST /api/login` — `{ email, password, device_name }` → `{ token, device_id }`
- `POST /api/devices` · `GET /api/devices` (Bearer)
- `POST /api/notes` — `{ title }` · `GET /api/notes` — propias y compartidas
- `GET /api/notes/:id` — metadatos + `body` materializado · `PATCH` (título) ·
  `DELETE` (solo owner, borrado lógico)
- `POST /api/notes/:id/share` — `{ user_id | user_email, role }` (`editor`/`viewer`,
  solo owner) · `DELETE /api/notes/:id/share/:user_id`
- `POST /api/import` — `{ title, body }` divide el body en líneas (migración
  offline → server) · `GET /api/notes/:id/export` — concatena líneas vivas
  (server → offline)

### Roles

| Rol | Permisos |
|-----|----------|
| `owner` | leer, editar, compartir, borrar la nota |
| `editor` | leer y editar |
| `viewer` | unirse a la sesión y mirar; no puede enviar operaciones |

## Relay de sincronización de dispositivos (`GET /api/sync`)

Además del canal colaborativo, el servidor implementa el relay WebSocket que habla
el `DbBackend` actual de keeplin-core (handshake `{"type":"auth","token"}` + sobres
`{"type":"changes",…}`), con journal persistente, catch-up en diferido por cursor de
dispositivo y dedupe de reintentos. Sirve para sincronizar los dispositivos de un
mismo usuario mientras el modo colaborativo llega al daemon. Un login (un token) por
dispositivo.

## Requisitos

- Rust >= 1.75
- PostgreSQL 16 (o usar Docker Compose)

## Arranque rápido

```bash
docker compose up -d        # PostgreSQL
cp .env.example .env        # cambia JWT_SECRET en producción
cargo run
```

El servidor escucha en `http://localhost:3000`.

## Variables de entorno

| Variable | Por defecto | Descripción |
|----------|-------------|-------------|
| `PORT` | `3000` | Puerto HTTP/WS |
| `DATABASE_URL` | — (obligatoria) | Conexión a PostgreSQL |
| `JWT_SECRET` | valor de desarrollo | Secreto de firma de tokens; cámbialo |
| `TOKEN_TTL_DAYS` | `365` | Vida de los tokens |
| `CHANGES_RETENTION_DAYS` | `0` (desactivado) | Poda del journal del relay |
| `RUST_LOG` | `info` | Nivel de log |

En producción termina TLS en un reverse proxy (`wss://`) — el token viaja en la
query / primer frame del WebSocket.

## Tests

```bash
export DATABASE_URL=postgres://keeplin:keeplin@127.0.0.1:5432/keeplin
cargo test
```

- `tests/collab.rs` — el protocolo colaborativo de extremo a extremo: Join/Welcome,
  propagación de ops con `server_seq`, resolución determinista de ediciones
  concurrentes, replays ignorados, Move, presencia con cursores, roles (viewer sin
  escritura, extraños sin acceso), suplantación de `last_writer` rechazada e
  import/export.
- `tests/integration.rs` — el relay de dispositivos con el cliente real
  (`DbBackend` de keeplin-core).

La CI (GitHub Actions) ejecuta fmt, check, tests contra Postgres 16 y clippy.
