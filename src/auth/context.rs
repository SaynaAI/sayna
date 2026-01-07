/// Authenticated client information
///
/// Inserted into request extensions by the auth middleware after successful
/// validation. Handlers can extract this via `Extension<Auth>` to access
/// the authenticated client's identity.
///
/// This struct is designed to be extensible - new fields can be added
/// as the auth service evolves.
///
/// # Example
/// ```rust,ignore
/// use axum::Extension;
/// use sayna::auth::Auth;
///
/// async fn my_handler(
///     Extension(auth): Extension<Auth>,
/// ) -> impl IntoResponse {
///     if let Some(id) = &auth.id {
///         tracing::info!("Request from client: {}", id);
///     }
///     // ...
/// }
/// ```
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Auth {
    /// The authenticated client/project identifier (e.g., "project1", "client-a")
    #[serde(default)]
    pub id: Option<String>,
    // Future fields can be added here, e.g.:
    // pub org_id: Option<String>,
    // pub permissions: Vec<String>,
    // pub metadata: Option<serde_json::Value>,
}

impl Auth {
    /// Create a new Auth with the given id
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: Some(id.into()),
        }
    }

    /// Create an empty Auth (no id)
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the effective auth id, treating empty/whitespace-only as None.
    ///
    /// Use this method when you need to guard auth-aware operations:
    /// - Room access checks should only run when this returns `Some`
    /// - Metadata updates should only write auth_id when this returns `Some`
    ///
    /// This ensures consistent behavior across all endpoints: empty `auth.id`
    /// is treated the same as unauthenticated mode (no auth.id).
    ///
    /// # Returns
    /// * `Some(&str)` - The trimmed auth id when present and non-empty
    /// * `None` - When auth.id is None, empty, or whitespace-only
    ///
    /// # Examples
    /// ```
    /// use sayna::auth::Auth;
    ///
    /// let auth = Auth::new("project1");
    /// assert_eq!(auth.effective_id(), Some("project1"));
    ///
    /// let empty = Auth::empty();
    /// assert_eq!(empty.effective_id(), None);
    ///
    /// let whitespace = Auth { id: Some("  ".to_string()) };
    /// assert_eq!(whitespace.effective_id(), None);
    /// ```
    pub fn effective_id(&self) -> Option<&str> {
        match &self.id {
            Some(id) => {
                let trimmed = id.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ effective_id() tests ============

    #[test]
    fn test_effective_id_with_valid_id() {
        let auth = Auth::new("project1");
        assert_eq!(auth.effective_id(), Some("project1"));
    }

    #[test]
    fn test_effective_id_with_empty_auth() {
        let auth = Auth::empty();
        assert_eq!(auth.effective_id(), None);
    }

    #[test]
    fn test_effective_id_with_empty_string() {
        let auth = Auth {
            id: Some("".to_string()),
        };
        assert_eq!(auth.effective_id(), None);
    }

    #[test]
    fn test_effective_id_with_whitespace_only() {
        let auth = Auth {
            id: Some("   ".to_string()),
        };
        assert_eq!(auth.effective_id(), None);
    }

    #[test]
    fn test_effective_id_with_tabs_and_newlines() {
        let auth = Auth {
            id: Some("\t\n ".to_string()),
        };
        assert_eq!(auth.effective_id(), None);
    }

    #[test]
    fn test_effective_id_trims_whitespace() {
        let auth = Auth {
            id: Some("  tenant-a  ".to_string()),
        };
        assert_eq!(auth.effective_id(), Some("tenant-a"));
    }

    #[test]
    fn test_effective_id_preserves_internal_whitespace() {
        let auth = Auth {
            id: Some("tenant a".to_string()),
        };
        assert_eq!(auth.effective_id(), Some("tenant a"));
    }
}
