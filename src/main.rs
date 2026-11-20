use axum::routing::get;
use axum::{extract::{DefaultBodyLimit, State}, http::StatusCode, routing::post, Router};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::fs;
use anyhow::Result;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{error, info, Level};
use tracing_subscriber::{layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter};

use crate::config::CONFIG;
use crate::runtime::Runtime;
use crate::engine::{delete_image, download, upload};

mod auth;
mod config;
mod engine;
mod error;
mod runtime;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    /// Settings that can be reloaded while running
    runtime: Arc<Runtime>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // The log filter is replaced once the runtime config is loaded, and on every reload
    let (log_filter, log_handle) = reload::Layer::new(EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(log_filter)
        .with(tracing_subscriber::fmt::layer().with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout())))
        .init();

    let runtime = Arc::new(Runtime::load(CONFIG.config_file.as_deref(), log_handle).expect("Couldn't load runtime config"));
    match &CONFIG.config_file {
        Some(path) => runtime.clone().watch(path.clone()),
        None => info!("CONFIG_FILE is not set, using default runtime settings"),
    }

    // Fail at startup, not on the first request, if the JWT secret is unusable
    std::sync::LazyLock::force(&auth::DECODING_KEY);

    // Connect to the database
    let db = PgPool::connect(&CONFIG.db_url).await.expect("Couldn't connect to database");

    sqlx::migrate!("./migrations").run(&db).await.expect("Migrations failed");
    
    // Prepare the upload directory
    fs::create_dir_all(&CONFIG.upload_dir).await.expect("Couldn't create upload directory");
    
    let app_state = AppState {
        db: db.clone(),
        runtime,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/", post(upload))
        .route("/{image_id}", get(download).delete(delete_image))
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(1024 * 1024 * 5)) // 5 MB
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        );

    let listener = tokio::net::TcpListener::bind(&CONFIG.bind_addr).await.expect("Couldn't bind to tcp listener");
    info!("Listening on {}", CONFIG.bind_addr);

    // In-flight requests are allowed to finish before the server exits
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");

    db.close().await;
    info!("Shut down");
    Ok(())
}

/// Reports healthy when the database is reachable.
async fn health(State(state): State<AppState>) -> StatusCode {
    match sqlx::query("select 1").execute(&state.db).await {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            error!("Health check failed: {}", e);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

/// Resolves on Ctrl+C or, on Unix, SIGTERM (what container orchestrators send).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("Couldn't listen for Ctrl+C");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Couldn't listen for SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Shutdown signal received, finishing in-flight requests");
}
