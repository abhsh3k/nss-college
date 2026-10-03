use std::sync::OnceLock;

use argon2::{
    password_hash::{
        rand_core::{OsRng, RngCore},
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
    },
    Argon2,
};

use crate::error::{internal, AppError};

pub const MIN_LENGTH: usize = 10;

pub fn hash(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(internal)
}

pub fn verify(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// A real hash used to burn the same CPU time when the login name does not exist,
/// so response time does not reveal which accounts exist.
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash("not-a-real-password").unwrap_or_default())
}

/// Argon2 is deliberately slow, so run it off the async worker threads.
pub async fn verify_blocking(password: String, stored_hash: String) -> Result<bool, AppError> {
    tokio::task::spawn_blocking(move || verify(&password, &stored_hash))
        .await
        .map_err(internal)
}

pub async fn verify_dummy(password: String) -> Result<(), AppError> {
    let dummy = dummy_hash().to_string();
    verify_blocking(password, dummy).await.map(|_| ())
}

pub async fn hash_blocking(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || hash(&password))
        .await
        .map_err(internal)?
}

/// A readable one-time password for new accounts (no 0/O/1/l/I lookalikes).
pub fn temp_password() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 12];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect()
}

/// A readable one-time password for new or reset accounts (no look-alike characters).
pub fn temporary() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 12];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}
