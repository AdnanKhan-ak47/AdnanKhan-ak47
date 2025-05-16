use crate::{
    exports::{get_auth_headers, OWNER_ID, USER_NAME},
    utility::{query_count, simple_request},
};
use dotenvy::dotenv;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
};
use xmltree::{Element, XMLNode};

pub fn user_getter(username: &str) -> Result<(String, String), Box<dyn Error>> {
    // Count the query usage
    query_count("user_getter");

    // Graphql query
    let query = r#"
        query($login: String!){
            user(login: $login){
            id
            createdAt
            }
        }
    "#;

    let variables = json!({ "login": username });

    let response = simple_request("user_getter", query, variables)?;

    let json: Value = response.json()?;
    let user = &json["data"]["user"];

    let id = user["id"].as_str().unwrap_or_default().to_string();
    let created_at = user["createdAt"].as_str().unwrap_or_default().to_string();

    Ok((id, created_at))
}

pub fn recursive_loc(
    owner: &str,
    repo_name: &str,
    data: &mut Value,
    cache_comment: &str,
    addition_total: usize,
    deletion_total: usize,
    my_commits: usize,
    cursor: Option<String>,
) -> Result<(usize, usize, usize), Box<dyn Error>> {
    query_count("recursive_loc");

    // GraphQL query with pagination
    let query = r#"
        query ($repo_name: String!, $owner: String!, $cursor: String) {
            repository(name: $repo_name, owner: $owner) {
                defaultBranchRef {
                    target {
                        ... on Commit {
                            history(first: 100, after: $cursor) {
                                totalCount
                                edges {
                                    node {
                                        ... on Commit {
                                            committedDate
                                        }
                                        author {
                                            user {
                                                id
                                            }
                                        }
                                        deletions
                                        additions
                                    }
                                }
                                pageInfo {
                                    endCursor
                                    hasNextPage
                                }
                            }
                        }
                    }
                }
            }
        }
    "#;

    let variables = json!({
        "repo_name": repo_name,
        "owner": owner,
        "cursor": cursor
    });

    let client = reqwest::blocking::Client::new();
    let response = client
        .post("https://api.github.com/graphql")
        .headers(get_auth_headers())
        .json(&json!({
            "query": query,
            "variables": variables,
        }))
        .send()?;

    let status = response.status();
    let json: Value = response.json()?;

    if status == 200 {
        let repo = &json["data"]["repository"]["defaultBranchRef"];
        if !repo.is_null() {
            let history = &repo["target"]["history"];
            return loc_counter_one_repo(
                owner,
                repo_name,
                data,
                cache_comment,
                history,
                addition_total,
                deletion_total,
                my_commits,
            );
        } else {
            return Ok((0, 0, 0));
        }
    }

    force_close_file(data, cache_comment)?;

    if status == 403 {
        return Err("Too many arguments! You've hit Github's Anti-abuse limit".into());
    }

    // Generic error
    Err(format!("recursive_loc() failed with status {}: {:?}", status, json).into())
}

pub fn loc_counter_one_repo(
    owner: &str,
    repo_name: &str,
    data: &mut Value,
    cache_comment: &str,
    history: &Value,
    mut addition_total: usize,
    mut deletion_total: usize,
    mut my_commits: usize,
) -> Result<(usize, usize, usize), Box<dyn Error>> {
    if let Some(edges) = history["edges"].as_array() {
        for node in edges {
            let author_id = &node["node"]["author"]["user"]["id"];
            if !author_id.is_null() && author_id == OWNER_ID.get().unwrap() {
                my_commits += 1;
                addition_total += node["node"]["additions"].as_u64().unwrap_or(0) as usize;
                deletion_total += node["node"]["deletions"].as_u64().unwrap_or(0) as usize;
            }
        }

        let has_next_page = history["pageInfo"]["hasNextPage"]
            .as_bool()
            .unwrap_or(false);
        if has_next_page && !edges.is_empty() {
            let end_cursor = history["pageInfo"]["endCursor"]
                .as_str()
                .map(|s| s.to_string());
            return recursive_loc(
                owner,
                repo_name,
                data,
                cache_comment,
                addition_total,
                deletion_total,
                my_commits,
                end_cursor,
            );
        }
    }
    // Base case: no more pages
    Ok((addition_total, deletion_total, my_commits))
}

pub fn loc_query(
    owner_affiliation: Vec<String>,
    comment_size: usize,
    force_cache: bool,
    cursor: Option<String>,
    mut edges: Vec<Value>,
) -> Result<(i32, i32, i32, bool), Box<dyn Error>> {
    query_count("loc_query");

    let query = r#"
        query ($owner_affiliation: [RepositoryAffiliation], $login: String!, $cursor: String) {
            user(login: $login) {
                repositories(first: 60, after: $cursor, ownerAffiliations: $owner_affiliation) {
                    edges {
                        node {
                            ... on Repository {
                                nameWithOwner
                                defaultBranchRef {
                                    target {
                                        ... on Commit {
                                            history {
                                                totalCount
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    pageInfo {
                        endCursor
                        hasNextPage
                    }
                }
            }
        }
    "#;

    let variables = json!({
        "owner_affiliation": owner_affiliation,
        "login": *USER_NAME,
        "cursor": cursor,
    });

    let response = simple_request("loc_query", query, variables)?;
    let json_data: Value = response.json()?;

    let repo_data = &json_data["data"]["user"]["repositories"];
    let new_edges = repo_data["edges"].as_array().unwrap_or(&vec![]).clone();
    edges.extend(new_edges);

    let has_next = repo_data["pageInfo"]["hasNextPage"]
        .as_bool()
        .unwrap_or(false);
    if has_next {
        let end_cursor = repo_data["pageInfo"]["endCursor"]
            .as_str()
            .map(|s| s.to_string());
        return loc_query(owner_affiliation, comment_size, force_cache, cursor, edges);
    }

    cache_builder(
        &edges,
        comment_size,
        force_cache,
        0,
        0,
        USER_NAME.clone().as_str(),
    )
}

pub fn commit_counter(comment_size: usize) -> Result<usize, Box<dyn Error>> {
    let hash = Sha256::digest(USER_NAME.as_bytes());
    let filename = format!("cache/{}.txt", hex::encode(hash));

    let file = File::open(&filename)?;
    let reader = BufReader::new(file);

    let mut total_commits = 0;

    for (index, line_result) in reader.lines().enumerate() {
        let line = line_result?;
        if index < comment_size {
            continue; // skip comment block
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() > 3 {
            if let Ok(commit_count) = parts[2].parse::<i32>() {
                total_commits += commit_count;
            }
        }
    }

    Ok(total_commits as usize)
}

pub fn graph_repos_stars(
    count_type: &str,
    owner_affiliation: Vec<String>,
    cursor: Option<String>,
    user_name: &str,
    github_token: &str,
) -> Result<usize, Box<dyn Error>> {
    let query = r#"
        query ($owner_affiliation: [RepositoryAffiliation], $login: String!, $cursor: String) {
            user(login: $login) {
                repositories(first: 100, after: $cursor, ownerAffiliations: $owner_affiliation) {
                    totalCount
                    edges {
                        node {
                            ... on Repository {
                                nameWithOwner
                                stargazers {
                                    totalCount
                                }
                            }
                        }
                    }
                    pageInfo {
                        endCursor
                        hasNextPage
                    }
                }
            }
        }
    "#;

    let variables = json!({
        "owner_affiliation": owner_affiliation,
        "login": user_name,
        "cursor": cursor
    });

    let response = simple_request("graph_repos_stars", query, variables)?;
    let json: Value = response.json()?;

    let user = &json["data"]["user"];
    let repos = &user["repositories"];

    match count_type {
        "repos" => Ok(repos["totalCount"].as_i64().unwrap_or(0) as usize),
        "stars" => {
            let mut total_stars = 0;
            if let Some(edges) = repos["edges"].as_array() {
                for edge in edges {
                    total_stars += edge["node"]["stargazers"]["totalCount"]
                        .as_i64()
                        .unwrap_or(0);
                }
            }
            Ok(total_stars as usize)
        }
        _ => Err("Invalid Count type. Use \"repos\" or \"stars\".".into()),
    }
}

pub fn stats_getter() -> Result<Value, Box<dyn Error>> {
    query_count("stats_getter");

    let query = r#"
    query($login: String!) {
        user(login: $login) {
            pullRequests(first: 1) {
                totalCount
            }
            issues {
                totalCount
            }
        }
    }"#;

    let variables = json!({ "login": USER_NAME.to_string() });

    let response = simple_request("stats_getter", query, variables)?;
    let json: Value = response.json()?;

    // Instead of converting to HashMap, return the relevant user_data part as Value
    let user_data = &json["data"]["user"];

    Ok(user_data.clone()) // clone to return owned Value
}

pub fn add_archive() -> Result<[i32; 5], Box<dyn Error>> {
    let file = File::open("cache/repository_archive.txt")?;
    let reader = BufReader::new(file);

    let lines: Vec<String> = reader.lines().filter_map(Result::ok).collect();

    if lines.len() < 10 {
        return Ok([0, 0, 0, 0, 0]);
    }

    let data = &lines[7..lines.len() - 3];
    let mut added_loc = 0;
    let mut deleted_loc = 0;
    let mut added_commits = 0;
    let contributed_repos = data.len();

    for line in data {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            let my_commits = parts[2];
            let loc_add = parts[3].parse::<i32>().unwrap_or(0);
            let loc_del = parts[4].parse::<i32>().unwrap_or(0);

            added_loc += loc_add;
            deleted_loc += loc_del;

            if let Ok(val) = my_commits.parse::<i32>() {
                added_commits += val;
            }
        }
    }

    // Parse last line's 5th element, strip the trailing `,` if present
    if let Some(last_line) = lines.last() {
        let last_parts: Vec<&str> = last_line.split_whitespace().collect();
        if last_parts.len() >= 5 {
            let val_str = last_parts[4].trim_end_matches(',');
            if let Ok(val) = val_str.parse::<i32>() {
                added_commits += val;
            }
        }
    }

    Ok([
        added_loc,
        deleted_loc,
        added_loc - deleted_loc,
        added_commits,
        contributed_repos as i32,
    ])
}

pub fn cache_builder(
    edges: &Vec<Value>,
    comment_size: usize,
    force_cache: bool,
    mut loc_add: i32,
    mut loc_del: i32,
    user_name: &str,
) -> Result<(i32, i32, i32, bool), Box<dyn Error>> {
    let mut cached = true;

    let hash = Sha256::digest(user_name.as_bytes());
    let filename = format!("cache/{}.txt", hex::encode(hash));

    println!("cache_builder: Does this file exists? {}", &filename);
    let mut data: Vec<String> = if Path::new(&filename).exists() {
        BufReader::new(File::open(&filename)?)
            .lines()
            .filter_map(Result::ok)
            .collect()
    } else {
        let mut comments: Vec<String> = vec![];
        if comment_size > 0 {
            comments = vec![
                "This line is a comment block. Write whatever you want here."
                    .to_string();
                comment_size
            ];
        }
        fs::create_dir_all("cache")?;
        let mut f = File::create(&filename)?;
        for line in &comments {
            writeln!(f, "{}", line);
        }
        comments
    };

    if data.len().saturating_sub(comment_size) != edges.len() || force_cache {
        cached = false;
        flush_cache(edges, &filename, comment_size)?;
        data = BufReader::new(File::open(&filename)?)
            .lines()
            .filter_map(Result::ok)
            .collect();
    }

    let cache_comment = data[..comment_size].to_vec();
    let mut lines = data[comment_size..].to_vec();

    // right after you compute cache_comment: Vec<String>
    let cache_comment_str = cache_comment.join(""); // single String
                                                    // make a mutable JSON state to pass through
    let mut json_state = serde_json::json!({});

    for (index, edge) in edges.iter().enumerate() {
        if let Some(name_with_owner) = edge.pointer("/node/nameWithOwner").and_then(|v| v.as_str())
        {
            let repo_hash = Sha256::digest(name_with_owner.as_bytes());
            let hash_str = hex::encode(repo_hash);

            if let Some(line) = lines.get_mut(index) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.get(0) == Some(&hash_str.as_str()) {
                    let current_commit_count = edge
                        .pointer("/node/defaultBranchRef/target/history/totalCount")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);

                    let cached_commit_count = parts
                        .get(1)
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(0);

                    if current_commit_count != cached_commit_count {
                        let mut split = name_with_owner.split('/');
                        let owner = split.next().unwrap_or("");
                        let repo_name = split.next().unwrap_or("");

                        let (loc_add_new, loc_del_new, loc_total) = recursive_loc(
                            owner,
                            repo_name,
                            &mut json_state,
                            &cache_comment_str,
                            0,
                            0,
                            0,
                            None,
                        )?;
                        {
                            *line = format!(
                                "{} {} {} {} {}\n",
                                hash_str, current_commit_count, loc_total, loc_add_new, loc_del_new
                            );
                        }
                    }
                }
            }
        }
    }

    let mut file = File::create(&filename)?;
    for line in &cache_comment {
        writeln!(file, "{}", line);
    }

    for line in &lines {
        write!(file, "{}", line);
    }

    for line in &lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 {
            loc_add += parts[3].parse::<i32>().unwrap_or(0);
            loc_del += parts[4].parse::<i32>().unwrap_or(0);
        }
    }

    Ok((loc_add, loc_del, loc_add - loc_del, cached))
}

pub fn flush_cache(
    edges: &Vec<Value>,
    filename: &str,
    comment_size: usize,
) -> Result<(), Box<dyn Error>> {
    let preserved_comments = {
        println!("flush_cache: Does this file exists? {}", &filename);
        let file = File::open(filename)?;
        let mut reader = BufReader::new(file);
        let mut lines = Vec::new();

        for _ in 0..comment_size {
            let mut line = String::new();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                break; //EOF
            }
            lines.push(line);
        }
        lines
    };

    let mut file = File::create(filename)?;

    for line in &preserved_comments {
        file.write_all(line.as_bytes())?;
    }

    for node in edges.iter() {
        if let Some(name_with_owner) = node.pointer("/node/nameWithOwner").and_then(|v| v.as_str())
        {
            let mut hasher = Sha256::new();
            hasher.update(name_with_owner.as_bytes());
            let hash = hex::encode(hasher.finalize());

            let entry = format!("{} 0 0 0 0\n", hash);
            file.write_all(entry.as_bytes())?;
        }
    }

    Ok(())
}

pub fn force_close_file(
    data: &mut Value,
    cache_comment: &str,
) -> std::result::Result<(), Box<dyn Error>> {
    dotenv().ok();
    let mut hasher = Sha256::new();
    hasher.update(USER_NAME.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    let filename = format!("cache/{}.txt", hash);

    println!("force_close_file: Does this file exists? {}", &filename);
    let data_string = serde_json::to_string_pretty(data)?;

    let mut file = File::create(&filename)?;
    file.write_all(cache_comment.as_bytes())?;
    file.write_all(data_string.as_bytes())?;

    println!(
        "There was an error while writing to the cache file. The file {} has had the partial data saved and closed.",
        filename
    );

    Ok(())
}

/// Load an SVG file, overwrite the text content of specific <tspan> elements,
/// and write it back out.
pub fn svg_overwrite(
    filename: &str,
    commit_data: &str,
    star_data: &str,
    repo_data: &str,
    contrib_data: &str,
    stats_data: &serde_json::Value,
    loc_data: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    use std::fs;
    use xmltree::{Element, XMLNode};

    println!("svg overwrite called in path: {}", filename);
    let svg_content = fs::read_to_string(filename)?;
    let mut root = Element::parse(svg_content.as_bytes())?;

    println!("Did we reach here inside svg overwrite??");

    let mut tspans: Vec<*mut Element> = vec![];
    collect_tspans(&mut root, &mut tspans);

    // SAFETY: We ensure tspans are unique and safe to mutate after collection
    unsafe {
        if tspans.len() < 40 {
            panic!("Not enough <tspan> elements: found {}", tspans.len());
        }

        (*tspans[34]).children = vec![XMLNode::Text(repo_data.to_string())];
        (*tspans[36]).children = vec![XMLNode::Text(contrib_data.to_string())];
        (*tspans[38]).children = vec![XMLNode::Text(star_data.to_string())];
        (*tspans[40]).children = vec![XMLNode::Text(commit_data.to_string())];
        (*tspans[42]).children = vec![XMLNode::Text(stats_data["issues"].to_string())];
        (*tspans[44]).children = vec![XMLNode::Text(stats_data["prs"].to_string())];
        (*tspans[46]).children = vec![XMLNode::Text(loc_data[2].clone())];
        (*tspans[47]).children = vec![XMLNode::Text(format!("{}++", loc_data[0]))];
        (*tspans[48]).children = vec![XMLNode::Text(format!("{}--", loc_data[1]))];
    }

    let mut output = fs::File::create(filename)?;
    root.write(&mut output)?;

    Ok(())
}

/// Print the index and text content of every <tspan> in the SVG.
pub fn svg_element_getter(filename: &str) -> Result<(), Box<dyn Error>> {
    let mut svg_string = String::new();
    println!("Does this file exists? {}", &filename);

    File::open(filename)?.read_to_string(&mut svg_string)?;
    let root = Element::parse(svg_string.as_bytes())?;

    // Collect and print
    let mut index = 0;
    collect_and_print_tspans(&root, &mut index);

    Ok(())
}

/// Helper to recursively find and print each <tspan> element's text.
fn collect_and_print_tspans(elem: &Element, index: &mut usize) {
    if elem.name == "tspan" {
        // Gather its text children
        let text = elem
            .children
            .iter()
            .filter_map(|node| match node {
                XMLNode::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        println!("{}: {}", *index, text);
        *index += 1;
    }
    for child in &elem.children {
        if let XMLNode::Element(child_elem) = child {
            collect_and_print_tspans(child_elem, index);
        }
    }
}

fn collect_tspans(element: &mut Element, tspans: &mut Vec<*mut Element>) {
    for child in &mut element.children {
        if let XMLNode::Element(e) = child {
            if e.name == "tspan" {
                tspans.push(e as *mut Element);
            }
            collect_tspans(e, tspans);
        }
    }
}
