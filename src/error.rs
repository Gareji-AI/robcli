use serde::Serialize;

/// Contains only deliberately safe text, never source errors, headers, or cookies.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppError {
    pub code: &'static str,
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(skip)]
    pub hint: &'static str,
    #[serde(skip)]
    pub exit_code: u8,
}

impl AppError {
    pub fn new(code: &'static str, message: &'static str, hint: &'static str) -> Self {
        Self {
            code,
            message,
            http_status: None,
            retry_after_seconds: None,
            details: None,
            hint,
            exit_code: 2,
        }
    }

    pub fn network(code: &'static str, message: &'static str) -> Self {
        let mut error = Self::new(
            code,
            message,
            "Stop live testing. Investigate the failure; do not automatically retry, change accounts, or rotate proxies.",
        );
        error.exit_code = 3;
        error
    }

    /// HTTP and challenge failures are transport/response results (exit 3).
    /// Local validation stays on `new` (exit 2). No retry is implied.
    pub fn http(
        code: &'static str,
        message: &'static str,
        hint: &'static str,
        status: u16,
        retry_after_seconds: Option<u64>,
    ) -> Self {
        let mut error = Self::new(code, message, hint);
        error.http_status = Some(status);
        error.retry_after_seconds = retry_after_seconds;
        error.exit_code = 3;
        error
    }
}
