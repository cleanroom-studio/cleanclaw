use std::fmt;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CleanClawError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("rate limited")]
    RateLimited,

    #[error("upstream: {0}")]
    Upstream(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, CleanClawError>;

impl CleanClawError {
    pub fn http_status(&self) -> u16 {
        match self {
            CleanClawError::NotFound(_) => 404,
            CleanClawError::InvalidArgument(_) => 400,
            CleanClawError::Unauthorized => 401,
            CleanClawError::Forbidden => 403,
            CleanClawError::Conflict(_) => 409,
            CleanClawError::RateLimited => 429,
            CleanClawError::Upstream(_) => 502,
            CleanClawError::NotImplemented(_) => 501,
            CleanClawError::Internal(_) => 500,
        }
    }
}

impl From<std::io::Error> for CleanClawError {
    fn from(e: std::io::Error) -> Self {
        CleanClawError::Internal(format!("io: {e}"))
    }
}

impl From<serde_json::Error> for CleanClawError {
    fn from(e: serde_json::Error) -> Self {
        CleanClawError::InvalidArgument(format!("json: {e}"))
    }
}

impl From<sqlx::Error> for CleanClawError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => CleanClawError::NotFound("row".into()),
            other => CleanClawError::Internal(format!("db: {other}")),
        }
    }
}

#[derive(Debug)]
pub struct ApiError(pub CleanClawError);

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ApiError {}

impl From<CleanClawError> for ApiError {
    fn from(e: CleanClawError) -> Self {
        ApiError(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_includes_message() {
        let e = CleanClawError::NotFound("user u1".into());
        assert_eq!(e.to_string(), "not found: user u1");
    }

    #[test]
    fn error_invalid_argument_display() {
        let e = CleanClawError::InvalidArgument("missing foo".into());
        assert_eq!(e.to_string(), "invalid argument: missing foo");
    }

    #[test]
    fn error_unit_variants_display() {
        // Variants without a payload still render via thiserror.
        assert_eq!(CleanClawError::Unauthorized.to_string(), "unauthorized");
        assert_eq!(CleanClawError::Forbidden.to_string(), "forbidden");
        assert_eq!(CleanClawError::RateLimited.to_string(), "rate limited");
    }

    #[test]
    fn error_from_io() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        let e: CleanClawError = io.into();
        // io errors map to Internal (we don't surface a separate
        // Io variant — the From impl collapses to the Internal
        // bucket).
        assert!(matches!(e, CleanClawError::Internal(_)));
    }

    #[test]
    fn error_from_serde() {
        let bad: serde_json::Result<i32> = serde_json::from_str("not json");
        let e: CleanClawError = bad.unwrap_err().into();
        // serde errors map to InvalidArgument (the From impl
        // reports a "json: …" prefix so the dashboard can tell
        // parse failures from other invalid input).
        assert!(matches!(e, CleanClawError::InvalidArgument(_)));
    }

    #[test]
    fn api_error_wraps_cleanclaw_error() {
        let inner = CleanClawError::Conflict("dup".into());
        let api = ApiError(inner);
        assert_eq!(api.to_string(), "conflict: dup");
    }

    #[test]
    fn result_type_alias_is_usable() {
        // The `Result` alias is a std::result::Result with our
        // error type. Smoke-test the alias so downstream code can
        // rely on it.
        let ok: Result<i32> = Ok(42);
        assert_eq!(ok, Ok(42));
    }
}
