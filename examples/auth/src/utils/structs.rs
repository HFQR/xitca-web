use serde::Serialize;

/// Standard API response structure for all endpoints.
/// Provides consistent response format across the application.
#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}
