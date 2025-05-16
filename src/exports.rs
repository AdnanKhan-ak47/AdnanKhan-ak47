use dotenvy::dotenv;
use once_cell::sync::Lazy;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use std::env;

// Could be set once after querying user ID
pub static OWNER_ID: once_cell::sync::OnceCell<String> = once_cell::sync::OnceCell::new();

pub static USER_NAME: Lazy<String> = Lazy::new(|| {
    dotenv().ok();
    env::var("USER_NAME").expect("USER_NAME not found")
});

pub fn get_auth_headers() -> HeaderMap {
    dotenv().ok();
    let token = env::var("ACCESS_TOKEN").expect("Access Token not found");

    let mut headers = HeaderMap::new();

    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
    );

    headers.insert(USER_AGENT, HeaderValue::from_static("my_rust_app"));

    headers
}
