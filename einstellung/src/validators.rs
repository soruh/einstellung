//! Small dependency-free validators for common configuration invariants.
//!
//! These functions are intended for `#[config(validate = ...)]`. More application-specific
//! validation is usually clearer as a local function or closure.

/// Require a string to contain at least one byte.
///
/// # Errors
///
/// Returns `"value must not be empty"` when the string has no bytes.
pub fn non_empty(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        Err("value must not be empty")
    } else {
        Ok(())
    }
}

/// Require a string to contain at least one non-whitespace character.
///
/// # Errors
///
/// Returns `"value must not be blank"` when the string contains only whitespace or is empty.
pub fn non_blank(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() {
        Err("value must not be blank")
    } else {
        Ok(())
    }
}

/// Require a slice to contain at least one item.
///
/// # Errors
///
/// Returns `"value must not be empty"` when the slice contains no elements.
pub fn non_empty_slice<T>(value: &[T]) -> Result<(), &'static str> {
    if value.is_empty() {
        Err("value must not be empty")
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    missing_docs,
    clippy::missing_docs_in_private_items,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "Test fixtures model user input rather than library APIs."
)]
mod tests {
    use super::*;

    #[test]
    fn string_validators_distinguish_empty_blank_and_content() {
        assert!(non_empty("").is_err());
        assert!(non_empty(" ").is_ok());
        assert!(non_blank(" \t").is_err());
        assert!(non_blank(" value ").is_ok());
    }

    #[test]
    fn slice_validator_rejects_empty_values() {
        assert!(non_empty_slice::<u8>(&[]).is_err());
        assert!(non_empty_slice(&[1]).is_ok());
    }
}
