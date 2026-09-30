use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Moderator,
    User,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Moderator => "moderator",
            Role::User => "user",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "admin" => Role::Admin,
            "moderator" => Role::Moderator,
            _ => Role::User,
        }
    }
    pub fn has_permission(&self, required: &Role) -> bool {
        match required {
            Role::Admin => *self == Role::Admin,
            Role::Moderator => *self == Role::Admin || *self == Role::Moderator,
            Role::User => true,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: String,
}

impl From<User> for UserResponse {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            email: u.email,
            role: u.role,
            is_active: u.is_active,
            created_at: u.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub token_type: String,
    pub user: UserResponse,
}

#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub users: Vec<UserResponse>,
    pub total: i64,
    pub page: i64,
    pub per_page: i64,
}

#[derive(Debug, Deserialize)]
pub struct AdminUsersQuery {
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    /// `desc` (default, newest first) or `asc` (oldest first).
    pub order: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(length(min = 3, max = 30))]
    pub username: String,
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 128))]
    pub password: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(length(min = 1))]
    pub username: String,
    #[validate(length(min = 1))]
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRoleRequest {
    pub role: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_parsing_is_case_insensitive() {
        assert_eq!(Role::from_str("admin"), Role::Admin);
        assert_eq!(Role::from_str("Admin"), Role::Admin);
        assert_eq!(Role::from_str("MODERATOR"), Role::Moderator);
        assert_eq!(Role::from_str("user"), Role::User);
    }

    #[test]
    fn unknown_role_falls_back_to_user() {
        assert_eq!(Role::from_str("superuser"), Role::User);
        assert_eq!(Role::from_str(""), Role::User);
    }

    #[test]
    fn role_permission_hierarchy() {
        // Admin can do everything.
        assert!(Role::Admin.has_permission(&Role::Admin));
        assert!(Role::Admin.has_permission(&Role::Moderator));
        assert!(Role::Admin.has_permission(&Role::User));
        // Moderator sits in the middle.
        assert!(!Role::Moderator.has_permission(&Role::Admin));
        assert!(Role::Moderator.has_permission(&Role::Moderator));
        assert!(Role::Moderator.has_permission(&Role::User));
        // Plain users only pass user-level checks.
        assert!(!Role::User.has_permission(&Role::Admin));
        assert!(!Role::User.has_permission(&Role::Moderator));
        assert!(Role::User.has_permission(&Role::User));
    }

    #[test]
    fn role_display_and_serialization() {
        assert_eq!(Role::Admin.to_string(), "admin");
        assert_eq!(Role::Moderator.to_string(), "moderator");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
    }
}
