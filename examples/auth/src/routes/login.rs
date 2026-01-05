use crate::{
    AppState,
    utils::{response_builder::json_response, validator::flatten_errors},
};
use password_auth::verify_password;
use serde::{Deserialize, Serialize};
use validator::Validate;
use xitca_postgres::{Execute, Statement, iter::AsyncLendingIterator, types::Type};
use xitca_web::{
    error::Error,
    handler::{json::LazyJson, state::StateRef},
    http::{StatusCode, WebResponse},
};

#[derive(Deserialize, Validate)]
pub struct Login<'a> {
    #[validate(email(message = "Invalid email format"))]
    email: &'a str,
    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    password: &'a str,
}

#[derive(Serialize)]
pub struct UserData {
    id: String,
    name: String,
    email: String,
}

pub async fn login(
    StateRef(state): StateRef<'_, AppState>,
    lazy: LazyJson<Login<'_>>,
) -> Result<WebResponse, Error> {
    // 1. Attempt to deserialize the incoming JSON body
    let req = match lazy.deserialize() {
        Ok(data) => data,
        Err(e) => {
            println!(">>> JSON ERROR: Invalid format or struct mismatch: {:?}", e);

            return Ok(json_response::<()>(
                StatusCode::BAD_REQUEST,
                false,
                format!("Invalid request body: {}", e.to_string()),
                None,
            ));
        }
    };

    // 2. Run structural validation (email format, password length, etc.)
    if let Err(e) = req.validate() {
        let error_body = flatten_errors(e);

        return Ok(json_response(
            StatusCode::BAD_REQUEST,
            false,
            "Validation Failed",
            Some(error_body),
        ));
    }

    let Login { email, password } = req;

    // 3. Prepare the SQL statement for secure execution (prevents SQL injection)
    let stmt = Statement::named(
        "SELECT id, name, email, password FROM users WHERE email = $1",
        &[Type::TEXT],
    )
    .execute(state.db_client.client())
    .await
    .map_err(|e| Error::from(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

    // 4. Execute the query with the provided email
    let mut rows = stmt
        .bind([email])
        .query(state.db_client.client())
        .await
        .map_err(|e| Error::from(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

    // 5. Check if a user exists with that email
    if let Some(row) = rows
        .try_next()
        .await
        .map_err(|e| Error::from(std::io::Error::new(std::io::ErrorKind::Other, e)))?
    {
        // Extract data from the database row by index
        let user_id: String = row.get(0);
        let user_name: String = row.get(1);
        let user_email: String = row.get(2);
        let password_hash: String = row.get(3);

        // 6. Verify if the provided password matches the hashed password in the DB
        match verify_password(password, &password_hash) {
            Ok(_) => {
                let response = UserData {
                    id: user_id,
                    name: user_name,
                    email: user_email,
                };

                return Ok(json_response(
                    StatusCode::OK,
                    true,
                    "Login successful",
                    Some(response),
                ));
            }
            Err(_) => {
                // Password mismatch
                return Ok(json_response::<()>(
                    StatusCode::UNAUTHORIZED,
                    false,
                    "Invalid email or password",
                    None,
                ));
            }
        }
    } else {
        // 7. No user found with that email
        return Ok(json_response::<()>(
            StatusCode::UNAUTHORIZED,
            false,
            "Invalid email or password",
            None,
        ));
    }
}
