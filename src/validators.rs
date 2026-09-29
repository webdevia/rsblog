use crate::errors::{AppError, AppResult};
use validator::Validate;

pub fn validate_request<T: Validate>(req: &T) -> AppResult<()> {
    req.validate().map_err(|e| {
        let msgs: Vec<String> = e
            .field_errors()
            .into_iter()
            .flat_map(|(f, errs)| {
                errs.iter().map(move |err| {
                    format!(
                        "{}: {}",
                        f,
                        err.message.clone().unwrap_or_else(|| "invalid".into())
                    )
                })
            })
            .collect();
        AppError::Validation(msgs.join(", "))
    })
}

pub fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::RegisterRequest;

    #[test]
    fn slugify_basic_words() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(
            slugify("Asynchronous Programming in Rust"),
            "asynchronous-programming-in-rust"
        );
    }

    #[test]
    fn slugify_collapses_separators() {
        assert_eq!(slugify("Rust--Rocks!!"), "rust-rocks");
        assert_eq!(slugify("  spaced   out  "), "spaced-out");
        assert_eq!(slugify("snake_case_name"), "snake-case-name");
    }

    #[test]
    fn slugify_edge_cases() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify("already-slugified"), "already-slugified");
        assert_eq!(slugify("Version 2.0"), "version-2-0");
    }

    fn valid_register() -> RegisterRequest {
        RegisterRequest {
            username: "someuser".into(),
            email: "someuser@example.com".into(),
            password: "SecurePassword123".into(),
        }
    }

    #[test]
    fn validate_request_accepts_valid_input() {
        assert!(validate_request(&valid_register()).is_ok());
    }

    #[test]
    fn validate_request_rejects_short_username() {
        let mut req = valid_register();
        req.username = "ab".into();
        let err = validate_request(&req).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
        assert!(err.to_string().contains("username"));
    }

    #[test]
    fn validate_request_rejects_bad_email() {
        let mut req = valid_register();
        req.email = "not-an-email".into();
        let err = validate_request(&req).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn validate_request_rejects_short_password() {
        let mut req = valid_register();
        req.password = "short".into();
        let err = validate_request(&req).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }
}
