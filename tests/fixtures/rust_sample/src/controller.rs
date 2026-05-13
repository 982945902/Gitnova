use crate::auth::AuthService;

pub fn login_controller(email: &str, token: &str) -> bool {
    let service = AuthService::new("gitnova".to_string());
    service.validate(email, token)
}

