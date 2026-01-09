use crate::{
    AppState,
    utils::{error::DbError, structs::ApiResponse, validator::flatten_errors},
};
use cuid2;
use password_auth::generate_hash;
use serde::{Deserialize, Serialize};
use validator::Validate;
use xitca_postgres::{Execute, Statement, iter::AsyncLendingIterator, types::Type};
use xitca_web::{
    error::Error,
    handler::{json::Json, state::StateRef},
};

/// Registration request payload with validation rules.
#[derive(Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(length(min = 2, message = "Name must be at least 2 characters"))]
    name: String,
    #[validate(email(message = "Invalid email format"))]
    email: String,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    password: String,
}

/// Successful registration response.
#[derive(Serialize)]
pub struct RegisterResponse {
    user_id: String,
}

/// Registration endpoint handler.
/// Creates a new user with hashed password and collision-resistant ID.
pub async fn register(
    StateRef(state): StateRef<'_, AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<ApiResponse<RegisterResponse>>, Error> {
    // Validate input structure.
    // Using inspect_err for logging while preserving error handling.
    if let Err(e) = req
        .validate()
        .inspect_err(|e| eprintln!("Validation error: {:?}", e))
    {
        let error_body = flatten_errors(e);
        let response = ApiResponse {
            success: false,
            message: format!("Validation Failed: {:?}", error_body),
            data: None::<RegisterResponse>,
        };
        return Err(Error::from(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            serde_json::to_string(&response).unwrap(),
        )));
    }

    let RegisterRequest {
        name,
        email,
        password,
    } = req;

    // Generate collision-resistant ID using CUID2.
    let user_id = cuid2::create_id();

    // Hash password using password_auth (Argon2 by default).
    let hashed_password = generate_hash(password);

    // Get connection from pool.
    let conn = state.db_client.pool().get().await.map_err(DbError)?;

    // Prepare INSERT statement.
    // Bind parameters and execute query.
    let res = Statement::named(
        "INSERT INTO users (id, name, email, password) VALUES ($1, $2, $3, $4) RETURNING id",
        &[Type::TEXT, Type::TEXT, Type::TEXT, Type::TEXT],
    )
    .bind([&user_id, &name, &email, &hashed_password])
    .query(&conn)
    .await;

    match res {
        Ok(mut rows) => {
            let mut registered_id = String::new();
            if let Some(row) = rows.try_next().await.map_err(DbError)? {
                registered_id = row.get(0);
            }

            Ok(Json(ApiResponse {
                success: true,
                message: "Registration successful".to_string(),
                data: Some(RegisterResponse {
                    user_id: registered_id,
                }),
            }))
        }
        Err(e) => {
            let db_error = e.to_string();

            // Check for unique constraint violation (duplicate email).
            let (message, kind) = if db_error.contains("unique constraint") {
                (
                    "Email is already registered",
                    std::io::ErrorKind::AlreadyExists,
                )
            } else {
                ("Internal server error", std::io::ErrorKind::Other)
            };

            let response = ApiResponse {
                success: false,
                message: message.to_string(),
                data: None::<RegisterResponse>,
            };

            Err(Error::from(std::io::Error::new(
                kind,
                serde_json::to_string(&response).unwrap(),
            )))
        }
    }
}
