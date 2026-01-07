use std::collections::HashMap;
use validator::ValidationErrors;

/// Converts validator's nested ValidationErrors into a flat HashMap.
/// Makes validation errors easier to serialize and return to clients.
pub fn flatten_errors(e: ValidationErrors) -> HashMap<String, String> {
    let mut error_map = HashMap::new();
    for (field, errors) in e.field_errors() {
        if let Some(error) = errors.first() {
            let msg = error.message.as_deref().unwrap_or("Invalid value");
            error_map.insert(field.to_string(), msg.to_string());
        }
    }
    error_map
}
