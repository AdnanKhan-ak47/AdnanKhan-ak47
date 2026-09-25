use parking_lot::Mutex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    sync::LazyLock,
    thread,
    time::{Duration, Instant},
};

use crate::exports::AUTH_HEADERS;

const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const MAX_ATTEMPTS: u32 = 5;

pub static QUERY_COUNT: Mutex<BTreeMap<&'static str, usize>> = Mutex::new(BTreeMap::new());

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("failed to build HTTP client")
});

pub fn perf_counter<F, R>(func: F) -> (R, f64)
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let result = func();
    (result, start.elapsed().as_secs_f64())
}

pub fn print_time(label: &str, duration: f64) {
    let time = if duration > 1.0 {
        format!("{duration:.4} s")
    } else {
        format!("{:.4} ms", duration * 1000.0)
    };
    println!("{:<23}{time:>12}", format!("   {label}:"));
}

/// POST a GraphQL query, retrying transient failures (5xx, timeouts, dropped
/// connections) with exponential backoff: GitHub's GraphQL gateway regularly
/// returns 502/504 on heavy queries.
pub fn simple_request(
    func_name: &'static str,
    query: &str,
    variables: Value,
) -> Result<Value, Box<dyn Error>> {
    let payload = json!({ "query": query, "variables": variables });

    let mut attempt = 0;
    loop {
        attempt += 1;
        *QUERY_COUNT.lock().entry(func_name).or_insert(0) += 1;

        let retry_reason = match CLIENT
            .post(GRAPHQL_URL)
            .headers(AUTH_HEADERS.clone())
            .json(&payload)
            .send()
        {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    let json: Value = response.json()?;
                    if let Some(errors) = json.get("errors") {
                        return Err(format!("{func_name}: GraphQL errors: {errors}").into());
                    }
                    return Ok(json);
                }
                if !status.is_server_error() {
                    let body = response.text().unwrap_or_default();
                    return Err(format!("{func_name} failed with status {status}: {body}").into());
                }
                format!("status {status}")
            }
            Err(err) if err.is_timeout() || err.is_connect() || err.is_request() => err.to_string(),
            Err(err) => return Err(err.into()),
        };

        if attempt == MAX_ATTEMPTS {
            return Err(
                format!("{func_name} failed after {MAX_ATTEMPTS} attempts: {retry_reason}").into(),
            );
        }
        let delay = Duration::from_secs(5 << (attempt - 1));
        println!(
            "{func_name}: {retry_reason}; retrying in {}s ({attempt}/{MAX_ATTEMPTS})",
            delay.as_secs()
        );
        thread::sleep(delay);
    }
}
