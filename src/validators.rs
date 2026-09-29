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
