use argon2::password_hash::rand_core::{OsRng, RngCore};
use tower_sessions::Session;

use crate::error::{internal, AppError};

const KEY: &str = "csrf";

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The CSRF token for this session, created on first use.
pub async fn token(session: &Session) -> Result<String, AppError> {
    if let Some(existing) = session.get::<String>(KEY).await.map_err(internal)? {
        return Ok(existing);
    }
    let fresh = random_token();
    session.insert(KEY, &fresh).await.map_err(internal)?;
    Ok(fresh)
}

/// Forget the token (call after login so a new session gets a new token).
pub async fn rotate(session: &Session) -> Result<(), AppError> {
    session.remove::<String>(KEY).await.map_err(internal)?;
    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Every state-changing request must carry the session's token.
pub async fn verify(session: &Session, submitted: &str) -> Result<(), AppError> {
    let expected = session.get::<String>(KEY).await.map_err(internal)?;
    match expected {
        Some(e) if constant_time_eq(e.as_bytes(), submitted.as_bytes()) => Ok(()),
        _ => Err(AppError::Forbidden),
    }
}
