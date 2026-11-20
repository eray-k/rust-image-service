
# image-server

Small Rust service that accepts single-file image uploads and records them in a Postgres database. Built with [axum](https://github.com/tokio-rs/axum) & [sqlx](https://github.com/launchbadge/sqlx).

## The goals

- Use a hands-on project to sharpen my Rust skills by implementing a real file upload microservice.
- Build a lightweight, configurable file-upload server that can be migrated into projects easily.


## What it does

- Accepts multipart/form-data POST requests to `/` with field:
	- `image` (file, limited to ~2 MiB in the request struct)
- Upload and delete require `Authorization: Bearer <JWT>`, an HS256 token signed with `JWT_SECRET` (at least 32 bytes, used by this service only, since `aud`/`iss` aren't checked) whose `sub` claim is the user's UUID and which has an `exp` claim. Only the uploader can delete an image; deleting someone else's image returns `404`, same as a missing one. Download is public.
- Limits concurrent processing by a memory budget (each image costs roughly `width * height * (2 * bytes_per_pixel + 4)` bytes, so 12 for 8-bit RGBA and 20 for 16-bit RGBA) and a worker count. Requests that can't get capacity within the queue timeout get `503`.
- Validates the uploaded image file, checks for Magic Bytes.
- Processes the image file: by default center-crops to 1:1, downscales to 1024 px wide (never upscales) and converts to WebP.
- Images must be between 16x16 and 8192x8192 pixels.
- Only PNG, JPEG and WebP inputs are accepted; other decoders are not compiled in.
- Persists the processed image to the `UPLOAD_DIR` and inserts a record in the `image` table (id, user_id, filepath).

## Repo layout

- `src/` — application code
- `migrations/` — sqlx database migrations
- `.sqlx/` — offline query data, so the project builds without a database. Run `cargo sqlx prepare` after changing a query.
- `uploads/` — default directory used by the app for saved files (created at runtime)

## Requirements

- Rust (stable)
- PostgreSQL
- `cargo` and `sqlx` CLI if you want to run migrations locally

## Environment
Create a `.env` file according to `.env.template` file.

## Runtime configuration

Processing limits, output settings and the log level can be changed without a restart. Point `CONFIG_FILE` at a TOML file (see [`config.example.toml`](config.example.toml); every key is optional). The server re-reads it on `SIGHUP` and checks it for changes every 5 seconds. An invalid file is rejected with an error log and the current settings are kept; at startup an invalid file stops the server.

- Lowering limits never interrupts running jobs; the capacity is removed as they finish.
- Output settings apply to new uploads only.

## Build & Run

1. Ensure that `.env` is set correctly.
2. Ensure Postgres is running and the `DATABASE_URL` is reachable.
3. Run migrations (optional, since the app runs them at the startup):

```
cargo sqlx migrate run
```

4. Build and run the server:

```
cargo run
```

By default, the server binds to `127.0.0.1:7070`. Pass `--bind <addr>` to change it, e.g. `cargo run -- --bind 0.0.0.0:7070` (needed inside a container).

- `GET /health` returns `200` when the database is reachable, `503` otherwise.
- On `SIGTERM` or Ctrl+C the server stops accepting connections and lets in-flight requests finish.
- Logs go to stdout; set the level with `log.level` in the config file, or `RUST_LOG` when the file doesn't set it.

## Notes & TODOs

- S3 file uploads will be implemented.

- Make the processing limit adaptive (AIMD): grow the concurrent job limit while processing latency stays low and halve it when latency spikes, instead of relying only on the configured memory budget and worker count.

- ~~File validation uses Content-Type header only; for security, validate magic bytes and file size explicitly.~~

- The service will improved to handle/serve different type of files (`.pdf`, `.mp4`, `.csv`, etc.)
