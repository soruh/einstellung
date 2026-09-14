//! Small dependency-free validators for common configuration invariants.
//!
//! These functions are intended for `#[config(validate = ...)]`. More application-specific
//! validation is usually clearer as a local function or closure.

/// Require a string to contain at least one byte.
pub fn non_empty(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        Err("value must not be empty")
    } else {
        Ok(())
    }
}

/// Require a string to contain at least one non-whitespace character.
pub fn non_blank(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() {
        Err("value must not be blank")
    } else {
        Ok(())
    }
}

/// Require a slice to contain at least one item.
pub fn non_empty_slice<T>(value: &[T]) -> Result<(), &'static str> {
    if value.is_empty() {
        Err("value must not be empty")
    } else {
        Ok(())
    }
}

#[cfg(test)]
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
