use crate::errors::auth_error::AuthError;
use axum::http::{HeaderMap, Uri};

/// Extract authentication token from request.
///
/// Priority:
/// 1. `Authorization: Bearer <token>` header
/// 2. `api_key` query parameter from URL
///
/// If Authorization header is present but invalid, this function still falls
/// back to `api_key` to support clients that send both.
pub fn extract_auth_token(headers: &HeaderMap, uri: &Uri) -> Result<String, AuthError> {
    if let Some(header_value) = headers.get("authorization") {
        match header_value.to_str() {
            Ok(auth_header) => {
                if let Some(token) = auth_header.strip_prefix("Bearer ")
                    && !token.is_empty()
                {
                    return Ok(token.to_string());
                }
            }
            Err(_) => {
                if let Some(token) = extract_api_key_from_query(uri) {
                    return Ok(token);
                }
                return Err(AuthError::InvalidAuthHeader);
            }
        }

        if let Some(token) = extract_api_key_from_query(uri) {
            return Ok(token);
        }
        return Err(AuthError::InvalidAuthHeader);
    }

    if let Some(token) = extract_api_key_from_query(uri) {
        return Ok(token);
    }

    Err(AuthError::MissingAuthHeader)
}

fn extract_api_key_from_query(uri: &Uri) -> Option<String> {
    uri.query().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes()).find_map(|(key, value)| {
            if key == "api_key" {
                let token = value.trim();
                if token.is_empty() {
                    None
                } else {
                    Some(value.into_owned())
                }
            } else {
                None
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    #[test]
    fn extracts_token_from_authorization_header() {
        let request = Request::builder()
            .uri("/test?api_key=query-token")
            .header("authorization", "Bearer header-token")
            .body(())
            .unwrap();

        let token = extract_auth_token(request.headers(), request.uri()).unwrap();
        assert_eq!(token, "header-token");
    }

    #[test]
    fn falls_back_to_query_param_when_header_missing() {
        let request = Request::builder()
            .uri("/test?api_key=query-token")
            .body(())
            .unwrap();

        let token = extract_auth_token(request.headers(), request.uri()).unwrap();
        assert_eq!(token, "query-token");
    }

    #[test]
    fn falls_back_to_query_param_when_header_invalid() {
        let request = Request::builder()
            .uri("/test?api_key=query-token")
            .header("authorization", "InvalidFormat")
            .body(())
            .unwrap();

        let token = extract_auth_token(request.headers(), request.uri()).unwrap();
        assert_eq!(token, "query-token");
    }
}
