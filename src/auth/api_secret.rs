use crate::config::AuthApiSecret;
use subtle::ConstantTimeEq;

fn api_secret_matches(token: &str, secret: &str) -> bool {
    bool::from(token.as_bytes().ct_eq(secret.as_bytes()))
}

pub fn match_api_secret_id<'a>(token: &str, secrets: &'a [AuthApiSecret]) -> Option<&'a str> {
    secrets
        .iter()
        .find(|entry| api_secret_matches(token, &entry.secret))
        .map(|entry| entry.id.as_str())
}
