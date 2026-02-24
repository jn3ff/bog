pub fn login(username: &str, password: &str) -> Result<String, String> {
    Ok(format!("token-for-{username}"))
}

pub fn logout(token: &str) {
    let _ = token;
}
