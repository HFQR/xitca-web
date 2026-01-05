use crate::{
    AppState,
    utils::{response_builder::json_response, validator::flatten_errors},
};
use cuid2;
use password_auth::generate_hash;
use serde::Deserialize;
use serde_json::json;
use validator::Validate;
use xitca_postgres::{Execute, Statement, iter::AsyncLendingIterator, types::Type};
use xitca_web::{
    error::Error,
    handler::{json::LazyJson, state::StateRef},
    http::{StatusCode, WebResponse},
};

#[derive(Deserialize, Validate)]
pub struct RegisterRequest<'a> {
    #[validate(length(min = 2, message = "Name must be at least 2 characters"))]
    name: &'a str,
    #[validate(email(message = "Invalid email format"))]
    email: &'a str,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    password: &'a str,
}

pub async fn register(
    StateRef(state): StateRef<'_, AppState>,
    lazy: LazyJson<RegisterRequest<'_>>,
) -> Result<WebResponse, Error> {
    // --- Step 1: Deserialization ---
    // Extract JSON from the request body. Using LazyJson prevents automatic
    // error responses, allowing us to customize the error format.
    let req = match lazy.deserialize() {
        Ok(data) => data,
        Err(e) => {
            return Ok(json_response::<()>(
                StatusCode::BAD_REQUEST,
                false,
                format!("Invalid request body: {}", e.to_string()),
                None,
            ));
        }
    };

    // --- Step 2: Validation ---
    // Check if the input data meets our structural requirements.
    if let Err(e) = req.validate() {
        let error_body = flatten_errors(e);

        return Ok(json_response(
            StatusCode::BAD_REQUEST,
            false,
            "Validation Failed",
            Some(error_body),
        ));
    }

    let RegisterRequest {
        name,
        email,
        password,
    } = req;

    // --- Step 3: Identity & Security ---
    // Create a collision-resistant ID and hash the password for security.
    let user_id = cuid2::create_id();
    let hashed_password = generate_hash(password);

    // --- Step 4: Database Operations ---
    // Prepare the SQL insert statement.
    let stmt = Statement::named(
        "INSERT INTO users (id, name, email, password) VALUES ($1, $2, $3, $4) RETURNING id",
        &[Type::TEXT, Type::TEXT, Type::TEXT, Type::TEXT],
    )
    .execute(state.db_client.client())
    .await
    .map_err(|e| Error::from(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

    // Execute the query and bind our secure values.
    let res = stmt
        .bind([&user_id, name, email, &hashed_password])
        .query(state.db_client.client())
        .await;

    match res {
        Ok(mut rows) => {
            let mut registered_id = String::new();
            // Retrieve the ID generated/returned by the DB.
            if let Some(row) = rows
                .try_next()
                .await
                .map_err(|e| Error::from(std::io::Error::new(std::io::ErrorKind::Other, e)))?
            {
                registered_id = row.get(0);
            }

            Ok(json_response(
                StatusCode::CREATED,
                true,
                "Registration successful",
                Some(json!({"user_id": registered_id})),
            ))
        }
        Err(e) => {
            // --- Step 5: Specific Database Error Handling ---
            let db_error = e.to_string();

            // Check if the error is due to a duplicate email.
            let (status, message) = if db_error.contains("unique constraint") {
                (StatusCode::CONFLICT, "Email is already registered")
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            };

            Ok(json_response::<()>(status, false, message, None))
        }
    }
}
