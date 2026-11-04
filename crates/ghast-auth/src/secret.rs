//! A string that does not print itself.

use std::fmt;

/// A secret string (password, access token, refresh token).
///
/// `Debug` and `Display` print `[redacted]` so secrets do not end up in logs by accident. Use
/// [`SecretString::expose`] to get the actual value.
#[derive(Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "microsoft", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "microsoft", serde(transparent))]
pub struct SecretString(String);

impl SecretString {
    /// Wraps a secret value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the secret value.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Returns `true` if the secret is the empty string.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_are_redacted() {
        let s = SecretString::new("hunter2");
        assert_eq!(format!("{s:?}"), "[redacted]");
        assert_eq!(format!("{s}"), "[redacted]");
        assert_eq!(s.expose(), "hunter2");
    }
}
