use crate::{
    AppState,
    utils::{error::DbError, structs::ApiResponse, validator::flatten_errors},
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
    // Using inspect_err for logging while preserving error handling.
    if let Err(e) = req
        .validate()
        .inspect_err(|e| eprintln!("Validation error: {:?}", e))
    {
        let error_body = flatten_errors(e);
        let response = ApiResponse {
            success: false,
            message: format!("Validation Failed: {:?}", error_body),
            data: None::<UserData>,
        };
        return Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            serde_json::to_string(&response).unwrap(),
        )));
    }

    let Login { email, password } = req;

    // Get connection from pool - follows TechEmpower benchmark pattern.
    let conn = state.db_client.pool().get().await.map_err(DbError)?;

    // Prepare statement on the connection (prevents SQL injection).
    let mut rows = Statement::named(
        "SELECT id, name, email, password FROM users WHERE email = $1",
        &[Type::TEXT],
    )
    .bind([&email])
    .query(&conn)
    .await
    .map_err(DbError)?;

    if let Some(row) = rows.try_next().await.map_err(DbError)? {
        let user_id: String = row.get(0);
        let user_name: String = row.get(1);
        let user_email: String = row.get(2);
        let password_hash: String = row.get(3);

        // Verify password against stored hash using password_auth crate.
        match verify_password(password, &password_hash) {
            Ok(_) => Ok(Json(ApiResponse {
                success: true,
                message: "Login successful".to_string(),
                data: Some(UserData {
                    id: user_id,
                    name: user_name,
                    email: user_email,
                }),
            })),
            Err(_) => {
                // Return generic error message to prevent user enumeration.
                let response = ApiResponse {
                    success: false,
                    message: "Invalid email or password".to_string(),
                    data: None::<UserData>,
                };
                Err(Error::from(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    serde_json::to_string(&response).unwrap(),
                )))
            }
        }
    } else {
        // User not found - return same error message as invalid password.
        let response = ApiResponse {
            success: false,
            message: "Invalid email or password".to_string(),
            data: None::<UserData>,
        };
        Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            serde_json::to_string(&response).unwrap(),
        )))
    }
}
