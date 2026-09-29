use crate::errors::{AppError, AppResult};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, TokenData, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub username: String,
    pub role: String,
    pub exp: usize,
    pub iat: usize,
}

pub fn create_token(
    user_id: &str,
    username: &str,
    role: &str,
    secret: &str,
    exp_hours: i64,
) -> AppResult<String> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id.to_string(),
        username: username.to_string(),
        role: role.to_string(),
        exp: (now + Duration::hours(exp_hours)).timestamp() as usize,
        iat: now.timestamp() as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(AppError::Jwt)
}

pub fn verify_token(token: &str, secret: &str) -> AppResult<TokenData<Claims>> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(AppError::Jwt)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-that-is-long-enough-for-hs256";

    #[test]
    fn create_and_verify_roundtrip() {
        let token = create_token("user-1", "alice", "admin", SECRET, 24).unwrap();
        let claims = verify_token(&token, SECRET).unwrap().claims;
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.username, "alice");
        assert_eq!(claims.role, "admin");
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let token = create_token("user-1", "alice", "user", SECRET, 24).unwrap();
        let err = verify_token(&token, "a-different-secret-that-is-long").unwrap_err();
        assert!(matches!(err, AppError::Jwt(_)));
    }

    #[test]
    fn malformed_token_is_rejected() {
        let err = verify_token("not.a.jwt", SECRET).unwrap_err();
        assert!(matches!(err, AppError::Jwt(_)));
    }

    #[test]
    fn expired_token_is_rejected() {
        // Negative lifetime => `exp` already in the past.
        let token = create_token("user-1", "alice", "user", SECRET, -1).unwrap();
        let err = verify_token(&token, SECRET).unwrap_err();
        assert!(matches!(err, AppError::Jwt(_)));
    }
}
