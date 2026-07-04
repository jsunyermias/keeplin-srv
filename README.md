# keeplin-srv

Servidor multiusuario con edición colaborativa en tiempo real para Keeplin.

Escrito en Rust, usa PostgreSQL para la persistencia y un protocolo de operaciones
por **línea** sobre WebSocket. Reutiliza la resolución de conflictos por version
vectors de `keeplin-core` (`note_log::resolve`).

## Requisitos

- Rust >= 1.70
- PostgreSQL 16 (o usar Docker Compose)

## Arranque rápido

```bash
# 1. Levantar PostgreSQL
docker compose up -d

# 2. Copiar variables de entorno
cp .env.example .env

# 3. Compilar y ejecutar
cargo run
```

El servidor escucha en `http://localhost:3000`.

## API

### Auth

- `POST /api/register` — `{ email, password }`
- `POST /api/login` — `{ email, password, device_name }`

### Dispositivos

- `POST /api/devices` — `{ device_name }` (requiere Bearer token)

### Notas

- `GET /api/notes` — lista notas accesibles.
- `POST /api/notes` — `{ title }` — crea una nota.
- `GET /api/notes/:id` — metadatos de una nota.
- `PATCH /api/notes/:id` — `{ title }` — actualiza título.
- `DELETE /api/notes/:id` — borrado lógico.

### Compartir

- `POST /api/notes/:id/shares` — `{ user_email, role }` (`editor` o `viewer`).
- `GET /api/notes/:id/shares/:user_id`
- `DELETE /api/notes/:id/shares/:user_id`

### Import / export

- `POST /api/import` — `{ title, body }` — divide `body` en líneas.
- `GET /api/notes/:id/export` — concatena líneas con `\n`.

### WebSocket

- `GET /api/ws?token=<jwt>&note_id=<uuid>` — canal colaborativo.

Mensajes JSON con `type`: `InsertLine`, `UpdateLine`, `DeleteLine`, `MoveLines`,
`Cursor`, `Presence`. Al conectar el servidor envía un `Snapshot` con las líneas
actuales ordenadas por posición fraccionaria.

## Tests

```bash
# Asegúrate de que DATABASE_URL apunta a Postgres
export DATABASE_URL=postgres://keeplin:keeplin@127.0.0.1:5432/keeplin
cargo test
```

Los tests de integración usan `sqlx::test`, que crea bases de datos temporales
a partir de `DATABASE_URL`.
