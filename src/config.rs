// src/config.rs

use std::sync::LazyLock;

pub struct Config {
    pub db_url: String,
    pub upload_dir: String,
}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    // Simulate loading settings from a file or environment
    dotenvy::dotenv().expect("Error while loading environment variables");

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is not set!");
    let upload_dir = std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "./uploads".to_string());

    Config { db_url, upload_dir }
});
