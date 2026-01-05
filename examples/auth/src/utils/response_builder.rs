use serde::Serialize;
use serde_json::to_string;
use xitca_web::http::{Response, StatusCode, WebResponse};

use crate::utils::structs::ApiResponse;

pub fn json_response<T: Serialize>(
    status: StatusCode,
    success: bool,
    message: impl Into<String>,
    data: Option<T>,
) -> WebResponse {
    let body = ApiResponse {
        success,
        message: message.into(),
        data,
    };

    let json_body = to_string(&body)
        .unwrap_or_else(|_| r#"{"success":false,"message":"Serialisasi Gagal"}"#.to_string());

    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(json_body.into())
        .unwrap()
        .into()
}
