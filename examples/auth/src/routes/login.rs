use crate::{
    AppState,
    utils::{
        error::{AuthError, DbError, ValidationError},
        structs::ApiResponse,
    },
};
use password_auth::verify_password;
use serde::{Deserialize, Serialize};
use validator::Validate;
use xitca_postgres::{Execute, Statement, iter::AsyncLendingIterator, types::Type};
use xitca_web::{
    error::Error,
    handler::{json::Json, state::StateRef},
};

/// Login request payload with validation rules.
#[derive(Deserialize, Validate)]
pub struct Login {
    #[validate(email(message = "Invalid email format"))]
    email: String,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    password: String,
}

/// Successful login response containing user data.
#[derive(Serialize)]
pub struct UserData {
    id: String,
    name: String,
    email: String,
}

/// Login endpoint handler.
/// Validates credentials and returns user data on success.
pub async fn login(
    StateRef(state): StateRef<'_, AppState>,
    Json(req): Json<Login>,
) -> Result<Json<ApiResponse<UserData>>, Error> {
    // Validate input structure (email format, password length).
    req.validate().map_err(ValidationError)?;

    let Login { email, password } = req;

    // Prepare statement on the connection (prevents SQL injection).
    let mut rows = Statement::named(
        "SELECT id, name, email, password FROM users WHERE email = $1",
        &[Type::TEXT],
    )
    .bind([&email])
    .query(state.db_client.pool())
    .await
    .map_err(DbError)?;

    let row = rows.try_next().await.map_err(DbError)?.ok_or(AuthError::NotFound)?;

    let user_id: String = row.get(0);
    let user_name: String = row.get(1);
    let user_email: String = row.get(2);
    let password_hash: String = row.get(3);

    // Verify password against stored hash using password_auth crate.
    verify_password(password, &password_hash).map_err(|_| AuthError::PermissionDenied)?;

    Ok(Json(ApiResponse {
        success: true,
        message: "Login successful".to_string(),
        data: Some(UserData {
            id: user_id,
            name: user_name,
            email: user_email,
        }),
    }))
}
