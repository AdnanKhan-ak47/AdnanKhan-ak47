use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use std::{env, sync::LazyLock};

pub static USER_NAME: LazyLock<String> =
    LazyLock::new(|| env::var("USER_NAME").expect("USER_NAME not set"));

pub static AUTH_HEADERS: LazyLock<HeaderMap> = LazyLock::new(|| {
    let token = env::var("ACCESS_TOKEN").expect("ACCESS_TOKEN not set");
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("ACCESS_TOKEN is not a valid header value"),
    );
    headers.insert(USER_AGENT, HeaderValue::from_static("AdnanKhan-ak47-readme"));
    headers
});

/// Repos left out of every stat. `register` is a fork kept only for its free
/// subdomain; its ~59k upstream commits made LOC counting slow and flaky.
pub const EXCLUDED_REPOS: &[&str] = &["AdnanKhan-ak47/register"];
