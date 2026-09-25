mod exports;
mod query;
mod utility;

use std::error::Error;

use dotenvy::dotenv;
use exports::USER_NAME;
use query::{fetch_repos, loc_stats, stats_getter, svg_overwrite, user_getter};
use utility::{QUERY_COUNT, perf_counter, print_time};

/// Set to recompute LOC for every repo instead of trusting the cache.
const FORCE_REFRESH: bool = false;

fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    let user_name = USER_NAME.as_str();

    println!("Calculation times:");

    let (owner_id, user_time) = perf_counter(|| user_getter(user_name));
    let owner_id = owner_id?;
    print_time("account data", user_time);

    let (repos, repos_time) = perf_counter(|| fetch_repos(user_name));
    let repos = repos?;
    print_time("repositories", repos_time);

    let (loc, loc_time) = perf_counter(|| loc_stats(&repos, user_name, &owner_id, FORCE_REFRESH));
    let loc = loc?;
    print_time(&format!("LOC ({} refreshed)", loc.refreshed), loc_time);

    let (stats, stats_time) = perf_counter(|| stats_getter(user_name));
    let (issues, prs) = stats?;
    print_time("issues/prs stats", stats_time);

    let owned: Vec<_> = repos.iter().filter(|r| r.is_owned_by(user_name)).collect();
    let net_loc = loc.added as i64 - loc.deleted as i64;
    let values = [
        ("repo_data", owned.len().to_string()),
        ("contrib_data", repos.len().to_string()),
        ("star_data", owned.iter().map(|r| r.stars).sum::<u64>().to_string()),
        ("commit_data", loc.commits.to_string()),
        ("issue_data", issues.to_string()),
        ("pr_data", prs.to_string()),
        ("loc_data", with_commas(net_loc)),
        ("loc_add", format!("{}++", with_commas(loc.added as i64))),
        ("loc_del", format!("{}--", with_commas(loc.deleted as i64))),
    ];
    svg_overwrite("src/dark_mode.svg", &values)?;
    svg_overwrite("src/light_mode.svg", &values)?;

    print_time("Total function time", user_time + repos_time + loc_time + stats_time);

    let counts = QUERY_COUNT.lock();
    for (func_name, count) in counts.iter() {
        println!("{func_name} called {count} times");
    }
    println!("Total GitHub GraphQL API calls: {}", counts.values().sum::<usize>());

    Ok(())
}

/// `1234567` → `"1,234,567"`.
fn with_commas(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
