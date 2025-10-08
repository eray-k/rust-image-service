use axum::{extract::DefaultBodyLimit, routing::post, Router};
use sqlx::PgPool;
use tokio::fs;

use crate::config::CONFIG;
use crate::engine::upload;

mod config;
mod engine;
mod image_type;

#[derive(Clone)]
struct AppState {
    db: PgPool
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to the database
    let db = sqlx::postgres::PgPool::connect(&CONFIG.db_url).await.expect("Couldn't connect to database");

    sqlx::migrate!("./migrations").run(&db).await.expect("Migrations failed");
    
    // Prepare the upload directory
    fs::create_dir_all(&CONFIG.upload_dir).await.expect("Couldn't create upload directory");
    
    let app_state = AppState {db};

    let app = Router::new()
        .route("/", post(upload))
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(1024 * 1024 * 5)); // 5 MB

    
    let addr = "0.0.0.0:7070".to_string();
    let listener = tokio::net::TcpListener::bind(addr).await.expect("Couldn't bind to tcp listener");

    axum::serve(listener, app).await.unwrap();
    Ok(())
}
