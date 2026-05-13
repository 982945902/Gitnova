use crate::utils::normalize_email;

pub struct AuthService {
    issuer: String,
}

impl AuthService {
    pub fn new(issuer: String) -> Self {
        Self { issuer }
    }

    pub fn validate(&self, email: &str, token: &str) -> bool {
        let normalized = normalize_email(email);
        self.check_token(&normalized, token)
    }

    fn check_token(&self, email: &str, token: &str) -> bool {
        token.starts_with(&self.issuer) && email.contains('@')
    }
}

