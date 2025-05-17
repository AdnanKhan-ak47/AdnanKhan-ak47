mod exports;
mod query;
mod utility;

use std::{env, fs, sync::MutexGuard};

use dotenvy::dotenv;
use exports::{OWNER_ID, USER_NAME};
use query::{
    commit_counter, graph_repos_stars, loc_query, stats_getter, svg_overwrite, user_getter,
};
use utility::{formatter, perf_counter, query_count, QUERY_COUNT};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let user_name = env::var("USER_NAME").expect("USER_NAME not found!");
    let github_token = env::var("ACCESS_TOKEN").expect("ACCESS_TOKEN not found!");

    println!("Calculation times:");

    let (user_data, user_time) = {
        let (res, time) = perf_counter(|| user_getter(USER_NAME.as_str()));
        (res?, time)
    };
    let (owner_id, acc_date) = user_data;
    OWNER_ID.set(owner_id).expect("Owner id was already set");
    formatter("account data", user_time, None, 0);

    let affiliations = vec![
        "OWNER".to_string(),
        "COLLABORATOR".to_string(),
        "ORGANIZATION_MEMBER".to_string(),
    ];
    let comment_size = 7;
    let force_cache = false;
    let cursor = None;
    let edges = Vec::new();

    let (total_loc, loc_time) = {
        let (res, time) =
            perf_counter(|| loc_query(affiliations, comment_size, force_cache, cursor, edges));
        (res?, time)
    };

    if total_loc.3 {
        formatter("LOC (cached)", loc_time, None, 0);
    } else {
        formatter("LOC (no cache)", loc_time, None, 0);
    }

    let (commit_result, commit_time) = perf_counter(|| commit_counter(7));
    let commit_data = commit_result?;

    let (star_result, star_time) = perf_counter(|| {
        graph_repos_stars(
            "stars",
            vec!["OWNER".to_string()],
            None,
            &user_name,
            &github_token,
        )
    });
    let star_data = star_result?;

    let (repo_result, repo_time) = perf_counter(|| {
        graph_repos_stars(
            "repos",
            vec!["OWNER".to_string()],
            None,
            &user_name,
            &github_token,
        )
    });
    let repo_data = repo_result?;

    let (contrib_result, contrib_time) = perf_counter(|| {
        graph_repos_stars(
            "repos",
            vec![
                "OWNER".to_string(),
                "COLLABORATOR".to_string(),
                "ORGANIZATION_MEMBER".to_string(),
            ],
            None,
            &user_name,
            &github_token,
        )
    });
    let contrib_data = contrib_result?;

    let (stats_result, stats_time) = perf_counter(|| stats_getter());
    let stats_data = stats_result?;
    formatter("issues/prs stats", stats_time, None, 0);

    let commit_data = formatter("commit counter", commit_time, Some(commit_data), 0);
    let star_data = formatter("star counter", star_time, Some(star_data), 0);
    let repo_data = formatter("my repositories", repo_time, Some(repo_data), 0);
    let contrib_data = formatter("contributed repos", contrib_time, Some(contrib_data), 0);

    // Format added, deleted, and total LOC with commas
    // Convert to array or vector to iterate:
    let total_loc_arr = [total_loc.0, total_loc.1, total_loc.2, total_loc.3 as i32];
    let formatted_loc: Vec<String> = total_loc_arr
        .iter()
        .take(total_loc_arr.len() - 1)
        .map(|loc| format!("{:}", loc))
        .collect();

    svg_overwrite(
        "src/dark_mode.svg",
        commit_data.as_deref().unwrap_or(""),
        star_data.as_deref().unwrap_or(""),
        repo_data.as_deref().unwrap_or(""),
        contrib_data.as_deref().unwrap_or(""),
        &stats_data,
        &formatted_loc,
    )?;

    svg_overwrite(
        "src/light_mode.svg",
        commit_data.as_deref().unwrap_or(""),
        star_data.as_deref().unwrap_or(""),
        repo_data.as_deref().unwrap_or(""),
        contrib_data.as_deref().unwrap_or(""),
        &stats_data,
        &formatted_loc,
    )?;

    // Move cursor up to overwrite previous lines (ANSI escape sequences)
    print!(
        "\x1B[8F{:<21} {:>11.4} s \x1B[E\x1B[E\x1B[E\x1B[E\x1B[E\x1B[E\x1B[E\x1B[E\n",
        "Total function time:",
        user_time + loc_time + commit_time + star_time + repo_time + contrib_time + stats_time
    );

    // Print total GitHub GraphQL API calls and counts
    let query_count_guard: MutexGuard<_> = QUERY_COUNT.lock().unwrap();

    let total_calls: usize = query_count_guard.values().sum();

    for (funct_name, count) in query_count_guard.iter() {
        println!("{} called {} times", funct_name, count);
    }

    Ok(())
}
