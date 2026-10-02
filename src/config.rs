use std::{env, net::SocketAddr};

pub struct Config {
    pub addr: SocketAddr,
    pub upload_dir: String,
    pub database_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = env::var("PORT").unwrap_or_else(|_| "3000".into());
        let addr = format!("{host}:{port}")
            .parse()
            .expect("HOST/PORT must form a valid socket address");
        let upload_dir = env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".into());
        let database_url =
            env::var("DATABASE_URL").expect("DATABASE_URL must be set (copy .env.example to .env)");
        Self {
            addr,
            upload_dir,
            database_url,
        }
    }
}
