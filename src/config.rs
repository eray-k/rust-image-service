// src/config.rs

use std::path::PathBuf;
use std::sync::LazyLock;

/// HS256 keys must be at least as long as the hash output (RFC 7518, section 3.2).
const MIN_JWT_SECRET_LEN: usize = 32;

pub struct Config {
    pub db_url: String,
    pub upload_dir: String,
    pub jwt_secret: String,
    pub config_file: Option<PathBuf>,
    pub bind_addr: String,
}

/// Reads `--bind <addr>` (or `--bind=<addr>`) from the command line, defaulting to localhost.
fn bind_addr_arg() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(addr) = arg.strip_prefix("--bind=") {
            return addr.to_string();
        }
        if arg == "--bind" {
            return args.next().expect("--bind requires an address");
        }
    }
    "127.0.0.1:7070".to_string()
}

pub static CONFIG: LazyLock<Config> = LazyLock::new(|| {
    // A .env file is optional; in containers the variables come from the environment.
    dotenvy::dotenv().ok();

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is not set!");
    let upload_dir = std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "./uploads".to_string());
    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET is not set!");
    // An empty or short secret would let anyone forge tokens
    assert!(
        jwt_secret.len() >= MIN_JWT_SECRET_LEN,
        "JWT_SECRET must be at least {MIN_JWT_SECRET_LEN} bytes"
    );
    let config_file = std::env::var_os("CONFIG_FILE").map(PathBuf::from);

    let bind_addr = bind_addr_arg();

    Config { db_url, upload_dir, jwt_secret, config_file, bind_addr }
});
