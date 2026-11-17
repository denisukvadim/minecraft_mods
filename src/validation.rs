use crate::error::AppError;

/// Email gets a pragmatic check: one `@`, no whitespace, a dotted domain.
/// Deliverability is proven by the login flow, not by a regex here.
pub fn email(value: &str) -> Result<(), AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 254 {
        return Err(AppError::Validation(
            "email must be between 1 and 254 characters".to_owned(),
        ));
    }
    if value.chars().any(char::is_whitespace) {
        return Err(AppError::Validation(
            "email must not contain whitespace".to_owned(),
        ));
    }
    let Some((local, domain)) = value.split_once('@') else {
        return Err(AppError::Validation(
            "email must contain an @ character".to_owned(),
        ));
    };
    if local.is_empty() || local.len() > 64 {
        return Err(AppError::Validation(
            "email local part must be between 1 and 64 characters".to_owned(),
        ));
    }
    if domain.is_empty() || domain.contains('@') || !domain.contains('.') {
        return Err(AppError::Validation(
            "email domain must contain a dot".to_owned(),
        ));
    }
    if domain.starts_with('.') || domain.ends_with('.') {
        return Err(AppError::Validation(
            "email domain must not start or end with a dot".to_owned(),
        ));
    }
    Ok(())
}

pub fn username(value: &str) -> Result<(), AppError> {
    let length = value.chars().count();
    if !(3..=32).contains(&length) {
        return Err(AppError::Validation(
            "username must be between 3 and 32 characters".to_owned(),
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(AppError::Validation(
            "username may only contain letters, digits, '_', '-' and '.'".to_owned(),
        ));
    }
    Ok(())
}

pub fn password(value: &str) -> Result<(), AppError> {
    if !(8..=128).contains(&value.chars().count()) {
        return Err(AppError::Validation(
            "password must be between 8 and 128 characters".to_owned(),
        ));
    }
    Ok(())
}

/// Permission codes are dynamic, but keep the `resource:action` shape so
/// a typo in one endpoint does not silently create two unrelated rules.
pub fn permission_code(value: &str) -> Result<(), AppError> {
    let length = value.chars().count();
    if !(3..=100).contains(&length) || !value.contains(':') {
        return Err(AppError::Validation(
            "permission code must be 3-100 characters and contain ':' (for example mods:upload)"
                .to_owned(),
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | ':'))
    {
        return Err(AppError::Validation(
            "permission code may only contain lowercase letters, digits, '_', '-', '.' and ':'"
                .to_owned(),
        ));
    }
    Ok(())
}

pub fn role_name(value: &str) -> Result<(), AppError> {
    let length = value.chars().count();
    if !(2..=50).contains(&length) {
        return Err(AppError::Validation(
            "role name must be between 2 and 50 characters".to_owned(),
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(AppError::Validation(
            "role name may only contain letters, digits, '_' and '-'".to_owned(),
        ));
    }
    Ok(())
}

pub fn description(value: &str, max: usize) -> Result<(), AppError> {
    if value.chars().count() > max {
        return Err(AppError::Validation(format!(
            "description must be at most {max} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_reasonable_email() {
        assert!(email("steve@example.com").is_ok());
    }

    #[test]
    fn rejects_emails_without_a_dotted_domain() {
        assert!(email("steve@localhost").is_err());
        assert!(email("steve").is_err());
        assert!(email("steve@exa mple.com").is_err());
    }

    #[test]
    fn password_length_is_enforced() {
        assert!(password("short").is_err());
        assert!(password("long-enough").is_ok());
    }

    #[test]
    fn permission_codes_need_a_colon() {
        assert!(permission_code("mods:upload").is_ok());
        assert!(permission_code("mods.upload").is_err());
        assert!(permission_code("Mods:Upload").is_err());
    }
}
