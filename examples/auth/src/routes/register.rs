use crate::{
    AppState,
    utils::{
        error::{DbError, ValidationError},
        structs::ApiResponse,
    },
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
    req.validate().map_err(ValidationError)?;

    let RegisterRequest { name, email, password } = req;

    // Generate collision-resistant ID using CUID2.
    let user_id = cuid2::create_id();

    // Hash password using password_auth (Argon2 by default).
    let hashed_password = generate_hash(password);

    // Prepare INSERT statement.
    // Bind parameters and execute query.
    let mut rows = Statement::named(
        "INSERT INTO users (id, name, email, password) VALUES ($1, $2, $3, $4) RETURNING id",
        &[Type::TEXT, Type::TEXT, Type::TEXT, Type::TEXT],
    )
    .bind([&user_id, &name, &email, &hashed_password])
    .query(state.db_client.pool())
    .await
    .map_err(DbError)?;

    let row = rows.try_next().await.map_err(DbError)?.expect("sql must return id");
    let user_id = row.get("id");

    Ok(Json(ApiResponse {
        success: true,
        message: "Registration successful".to_string(),
        data: Some(RegisterResponse { user_id }),
    }))
}
