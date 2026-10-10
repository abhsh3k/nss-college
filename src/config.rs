use std::{env, net::SocketAddr};

pub struct Config {
    pub addr: SocketAddr,
    pub management_addr: SocketAddr,
    pub upload_dir: String,
    pub database_url: String,
    pub cookie_secure: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = env::var("PORT").unwrap_or_else(|_| "3000".into());
        let addr = format!("{host}:{port}")
            .parse()
            .expect("HOST/PORT must form a valid socket address");
        let management_host = env::var("MANAGEMENT_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let management_port = env::var("MANAGEMENT_PORT").unwrap_or_else(|_| "3001".into());
        let management_addr = format!("{management_host}:{management_port}")
            .parse()
            .expect("MANAGEMENT_HOST/MANAGEMENT_PORT must form a valid socket address");
        if !management_addr.ip().is_loopback() {
            panic!("MANAGEMENT_HOST must resolve to a loopback address");
        }
        let upload_dir = env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".into());
        let database_url =
            env::var("DATABASE_URL").expect("DATABASE_URL must be set (copy .env.example to .env)");
        let cookie_secure = env::var("COOKIE_SECURE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        Self {
            addr,
            management_addr,
            upload_dir,
            database_url,
            cookie_secure,
        }
    }
}
