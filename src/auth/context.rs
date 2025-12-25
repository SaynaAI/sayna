#[derive(Clone, Debug)]
pub enum AuthMethod {
    ApiSecret,
    Jwt,
}

#[derive(Clone, Debug)]
pub struct AuthContext {
    pub method: AuthMethod,
    pub api_secret_id: Option<String>,
}

impl AuthContext {
    pub fn api_secret(id: impl Into<String>) -> Self {
        Self {
            method: AuthMethod::ApiSecret,
            api_secret_id: Some(id.into()),
        }
    }

    pub fn jwt() -> Self {
        Self {
            method: AuthMethod::Jwt,
            api_secret_id: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.api_secret_id.as_deref()
    }
}
