
# image-server

Small Rust service that accepts single-file image uploads and records them in a Postgres database. Built with [axum](https://github.com/tokio-rs/axum) & [sqlx](https://github.com/launchbadge/sqlx).

## The goals

- Use a hands-on project to sharpen my Rust skills by implementing a real file upload microservice.
- Build a lightweight, configurable file-upload server that can be dropped into projects as an alternative to [hupload](https://github.com/ybizeul/hupload) and [DumbDrop](https://github.com/DumbWareio/DumbDrop), with the emphasis on configurability.



## What it does

- Accepts multipart/form-data POST requests to `/` with fields:
	- `user_id` (UUID)
	- `image` (file, limited to ~2 MiB in the request struct)
- Validates the uploaded file's Content-Type (jpeg/png/webp).
- Persists the uploaded file to the `UPLOAD_DIR` and inserts a record in the `image` table (id, user_id, filepath).

## Repo layout

- `src/` — application code
- `migrations/` — sqlx database migrations
- `uploads/` — default directory used by the app for saved files (created at runtime)

## Requirements

- Rust (stable)
- PostgreSQL
- `cargo` and `sqlx` CLI if you want to run migrations locally

## Environment
Create a `.env` file according to `.env.template` file.

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

By default the server binds to `0.0.0.0:7070`.

## Notes & TODOs

- S3 file uploads will be implemented.

- The code currently calls `unwrap()` on file persist and should be made robust (return error responses and remove partial files on failure).

- File validation uses Content-Type header only; for security, validate magic bytes and file size explicitly.

- The service will improved to handle/serve different type of files (`.pdf`, `.mp4`, `.csv`, etc.)
