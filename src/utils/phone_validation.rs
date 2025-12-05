//! Phone number validation utility for SIP transfers.
//!
//! Provides simple, permissive validation for phone numbers used in SIP call transfers.
//! Accepts digits with an optional leading `+` sign - no specific format enforcement
//! as the system needs to support international numbers, national numbers, and
//! internal extensions of any length.

/// Validates a phone number for SIP transfer operations.
///
/// # Validation Rules
///
/// - Must not be empty (after trimming whitespace)
/// - Must contain only digits (`0-9`), with an optional `+` prefix
/// - If `+` is present, it must be at the very beginning
///
/// # Returns
///
/// - `Ok(String)` - The normalized phone number with `tel:` prefix for SIP compatibility
/// - `Err(String)` - A human-readable error message
///
/// # Examples
///
/// ```
/// use sayna::utils::phone_validation::validate_phone_number;
///
/// // Valid international number
/// assert!(validate_phone_number("+1234567890").is_ok());
///
/// // Valid internal extension
/// assert!(validate_phone_number("0").is_ok());
///
/// // Invalid - contains letters
/// assert!(validate_phone_number("123abc").is_err());
/// ```
pub fn validate_phone_number(phone: &str) -> Result<String, String> {
    let trimmed = phone.trim();

    if trimmed.is_empty() {
        return Err("Phone number cannot be empty".to_string());
    }

    // Check for plus sign handling
    let (has_plus, digits_part) = if let Some(rest) = trimmed.strip_prefix('+') {
        (true, rest)
    } else {
        (false, trimmed)
    };

    // Plus with no digits is invalid
    if has_plus && digits_part.is_empty() {
        return Err("Phone number must contain at least one digit".to_string());
    }

    // Check that all remaining characters are digits
    for (i, ch) in digits_part.chars().enumerate() {
        if ch == '+' {
            return Err(format!(
                "Invalid character '+' at position {} - plus sign is only allowed at the beginning",
                i + if has_plus { 1 } else { 0 }
            ));
        }
        if !ch.is_ascii_digit() {
            return Err(format!(
                "Invalid character '{}' at position {} - only digits (0-9) and optional leading '+' are allowed",
                ch,
                i + if has_plus { 1 } else { 0 }
            ));
        }
    }

    // Return with tel: prefix for SIP compatibility
    Ok(format!("tel:{}", trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_international_numbers() {
        // Minimum with plus
        let result = validate_phone_number("+1");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:+1");

        // Standard international
        let result = validate_phone_number("+1234567890");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:+1234567890");

        // UK international
        let result = validate_phone_number("+447123456789");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:+447123456789");
    }

    #[test]
    fn test_valid_digits_only() {
        // Single digit (internal extension)
        let result = validate_phone_number("0");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:0");

        let result = validate_phone_number("1");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:1");

        // UK national format
        let result = validate_phone_number("07123456789");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:07123456789");

        // Internal extension
        let result = validate_phone_number("1234");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:1234");

        // All zeros
        let result = validate_phone_number("0000");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:0000");
    }

    #[test]
    fn test_invalid_empty() {
        let result = validate_phone_number("");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Phone number cannot be empty");
    }

    #[test]
    fn test_invalid_plus_only() {
        let result = validate_phone_number("+");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Phone number must contain at least one digit"
        );
    }

    #[test]
    fn test_invalid_plus_not_at_beginning() {
        let result = validate_phone_number("1+234");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("plus sign is only allowed at the beginning")
        );
    }

    #[test]
    fn test_invalid_contains_letters() {
        let result = validate_phone_number("+123abc");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid character 'a'"));

        let result = validate_phone_number("abc123");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid character 'a'"));
    }

    #[test]
    fn test_invalid_contains_dash() {
        let result = validate_phone_number("123-456");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid character '-'"));
    }

    #[test]
    fn test_invalid_contains_parentheses() {
        let result = validate_phone_number("(123)456");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid character '('"));
    }

    #[test]
    fn test_invalid_contains_space() {
        let result = validate_phone_number("123 456");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid character ' '"));
    }

    #[test]
    fn test_invalid_whitespace_only() {
        let result = validate_phone_number("  ");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Phone number cannot be empty");
    }

    #[test]
    fn test_whitespace_trimming() {
        // Leading and trailing whitespace should be trimmed
        let result = validate_phone_number("  +1234567890  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:+1234567890");

        let result = validate_phone_number("  0  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tel:0");
    }
}
