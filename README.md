# keeplin-srv

El servidor de sincronización para [Keeplin](https://github.com/jsunyermias/keeplin):
el *relay* de producción que el modo servidor de `keeplin-daemon` (`DbBackend`)
necesita y que el repo principal no incluye («*No production sync server ships in
this repo*»).

Escrito en Rust (axum + PostgreSQL). Implementa exactamente el protocolo WebSocket
que habla `keeplin-core`:

1. El daemon conecta y envía el handshake `{"type":"auth","token":"<jwt>"}`.
2. Empuja lotes `{"type":"changes","batch_id":…,"device_id":…,"changes":[Change…]}`.
3. El servidor le entrega lotes `{"type":"changes","changes":[Change…]}` — primero
   el *backlog* que ese dispositivo aún no ha visto y después, en vivo, los lotes
   del resto de dispositivos del mismo usuario. Nunca se hace eco al emisor.

Los `Change` se tratan como **JSON opaco**: el relay los persiste y los reenvía sin
interpretar el modelo de `keeplin-core`, así que la evolución del modelo del cliente
no exige migraciones del servidor. La resolución de conflictos (version vectors)
ocurre en los clientes, que aplican cada cambio de forma idempotente; por eso el
relay prefiere la entrega duplicada a la pérdida.

## Garantías

- **Persistencia**: cada lote aceptado se guarda en el journal (`changes`) antes del
  fan-out. Un dispositivo que estaba apagado recibe el backlog completo al conectar.
- **Cursor por dispositivo**: cada dispositivo tiene una marca de entrega durable
  que solo avanza tras un envío correcto.
- **Dedupe de reintentos**: `(batch_id, batch_index)` es único; el reenvío de un lote
  tras una reconexión no duplica filas.
- **Aislamiento por usuario**: los cambios solo viajan entre dispositivos de la misma
  cuenta.

## Requisitos

- Rust >= 1.75
- PostgreSQL 16 (o usar Docker Compose)

## Arranque rápido

```bash
# 1. Levantar PostgreSQL
docker compose up -d

# 2. Copiar variables de entorno
cp .env.example .env   # cambia JWT_SECRET en producción

# 3. Compilar y ejecutar
cargo run
```

El servidor escucha en `http://localhost:3000`.

## Conectar un keeplin-daemon

```bash
# 1. Crear cuenta (una vez)
curl -X POST http://localhost:3000/api/register \
  -H 'content-type: application/json' \
  -d '{"email":"yo@example.com","password":"secreto-largo"}'

# 2. Obtener un token PARA CADA dispositivo (¡no compartas el token entre máquinas!)
curl -X POST http://localhost:3000/api/login \
  -H 'content-type: application/json' \
  -d '{"email":"yo@example.com","password":"secreto-largo","device_name":"portatil"}'
# → { "token": "…", "device_id": "…" }
```

En el `config.toml` del daemon:

```toml
mode = "server"
server_url = "ws://localhost:3000/api/sync"   # wss:// en producción
auth_token = "<token del paso 2>"
```

El token identifica usuario **y** dispositivo: el relay lo usa para saber qué ha
recibido ya cada dispositivo y para no reenviarle sus propios cambios. Un login por
dispositivo.

## API

- `GET /health`
- `POST /api/register` — `{ email, password }`
- `POST /api/login` — `{ email, password, device_name }` → `{ token, device_id }`
- `POST /api/devices` — `{ device_name }` (Bearer token) → token para otro dispositivo
- `GET /api/devices` — lista los dispositivos de la cuenta (Bearer token)
- `GET /api/sync` — WebSocket de sincronización (handshake `auth` como primer frame)

## Variables de entorno

| Variable | Por defecto | Descripción |
|----------|-------------|-------------|
| `PORT` | `3000` | Puerto HTTP/WS |
| `DATABASE_URL` | — (obligatoria) | Conexión a PostgreSQL |
| `JWT_SECRET` | valor de desarrollo | Secreto de firma de tokens; cámbialo |
| `TOKEN_TTL_DAYS` | `365` | Vida de los tokens de dispositivo |
| `CHANGES_RETENTION_DAYS` | `0` (desactivado) | Poda del journal: borra cambios más antiguos que N días **ya entregados a todos los dispositivos** del usuario |
| `RUST_LOG` | `info` | Nivel de log |

En producción termina TLS en un reverse proxy y usa `wss://` — el token del
handshake viaja en claro dentro del WebSocket.

## Tests

```bash
export DATABASE_URL=postgres://keeplin:keeplin@127.0.0.1:5432/keeplin
cargo test
```

Los tests de integración usan `sqlx::test` (bases de datos temporales) y ejercitan
el servidor con el **cliente real**: dos instancias de `DbBackend` de `keeplin-core`
hablando el protocolo auténtico, incluida la entrega en diferido, el aislamiento
entre usuarios y el rechazo de tokens inválidos.

## Historial

La primera iteración de este repo era un servidor colaborativo por líneas con su
propio protocolo; se reemplazó por este relay para que el servidor hable exactamente
el protocolo de `keeplin-core` en vez de inventar otro. La versión TypeScript
anterior sigue en `legacy/`.
