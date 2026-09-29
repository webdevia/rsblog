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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let hash = hash_password("CorrectHorseBatteryStaple").unwrap();
        assert!(hash.starts_with("$argon2"));
        assert!(verify_password("CorrectHorseBatteryStaple", &hash).unwrap());
    }

    #[test]
    fn wrong_password_does_not_verify() {
        let hash = hash_password("the-right-password").unwrap();
        assert!(!verify_password("the-wrong-password", &hash).unwrap());
    }

    #[test]
    fn salts_are_unique_per_hash() {
        let a = hash_password("same-password").unwrap();
        let b = hash_password("same-password").unwrap();
        assert_ne!(a, b);
        assert!(verify_password("same-password", &a).unwrap());
        assert!(verify_password("same-password", &b).unwrap());
    }

    #[test]
    fn malformed_hash_returns_bad_request() {
        let err = verify_password("whatever", "not-a-valid-hash").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
