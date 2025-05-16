use once_cell::sync::Lazy;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::error::Error;
use std::sync::Mutex;
use std::time::Instant;

use crate::exports::get_auth_headers;

pub static QUERY_COUNT: Lazy<Mutex<HashMap<String, usize>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn query_count(func_id: &str) {
    let mut count = QUERY_COUNT.lock().unwrap();
    let entry = count.entry(func_id.to_string()).or_insert(0);
    *entry += 1;
}

pub fn perf_counter<F, R>(func: F) -> (R, f64)
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let result = func();
    let duration = start.elapsed().as_secs_f64();
    (result, duration)
}

pub fn formatter(
    query_type: &str,
    duration: f64,
    funct_return: Option<usize>,
    whitespace: usize,
) -> Option<String> {
    print!("{:<23}", format!("   {}:", query_type));

    if duration > 1.0 {
        println!("{:>12}", format!("{:.4} s", duration));
    } else {
        println!("{:>12}", format!("{:.4} ms", duration * 1000.0));
    }

    if let Some(value) = funct_return {
        Some(format!(
            "{:>width$}",
            format!("{:}", value),
            width = whitespace
        ))
    } else {
        None
    }
}

pub fn simple_request(
    func_name: &str,
    query: &str,
    variables: Value,
) -> Result<reqwest::blocking::Response, Box<dyn Error>> {
    let client = Client::new();
    let url = "https://api.github.com/graphql";

    let payload = json!({
        "query": query,
        "variables": variables,
    });

    let headers = get_auth_headers();

    let response = client
        .post(url)
        .headers(headers.clone())
        .json(&payload)
        .send()?;

    if response.status().is_success() {
        Ok(response)
    } else {
        Err(format!("{} failed with status {}", func_name, response.status()).into())
    }
}
