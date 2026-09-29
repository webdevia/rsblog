use crate::errors::{AppError, AppResult};
use argon2::{
    password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier},
    Argon2,
};

pub fn hash_password(password: &str) -> AppResult<String> {
    Ok(Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| AppError::BadRequest(format!("Hashing failed: {e}")))?
        .to_string())
}

pub fn verify_password(password: &str, hash: &str) -> AppResult<bool> {
    let parsed =
        PasswordHash::new(hash).map_err(|e| AppError::BadRequest(format!("Invalid hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}
