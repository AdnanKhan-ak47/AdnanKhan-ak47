use crate::{exports::EXCLUDED_REPOS, utility::simple_request};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    error::Error,
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
};
use xmltree::{Element, XMLNode};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

/// Number of free-form header lines at the top of the cache file.
const COMMENT_SIZE: usize = 7;

pub struct Repo {
    pub name_with_owner: String,
    pub stars: u64,
    pub total_commits: u64,
}

impl Repo {
    pub fn is_owned_by(&self, login: &str) -> bool {
        self.name_with_owner
            .split_once('/')
            .is_some_and(|(owner, _)| owner.eq_ignore_ascii_case(login))
    }
}

pub struct LocStats {
    pub added: u64,
    pub deleted: u64,
    pub commits: u64,
    pub refreshed: usize,
}

struct CacheEntry {
    total_commits: u64,
    my_commits: u64,
    added: u64,
    deleted: u64,
}

fn u64_at(value: &Value, pointer: &str) -> u64 {
    value.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
}

pub fn user_getter(username: &str) -> Result<String> {
    let query = r#"
        query($login: String!) {
            user(login: $login) { id }
        }
    "#;
    let json = simple_request("user_getter", query, json!({ "login": username }))?;
    json.pointer("/data/user/id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("user_getter: no user id for {username}").into())
}

/// Every repo the user owns, collaborates on, or belongs to via an org,
/// minus `EXCLUDED_REPOS`.
pub fn fetch_repos(username: &str) -> Result<Vec<Repo>> {
    let query = r#"
        query ($login: String!, $cursor: String) {
            user(login: $login) {
                repositories(first: 60, after: $cursor, ownerAffiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER]) {
                    nodes {
                        nameWithOwner
                        stargazerCount
                        defaultBranchRef {
                            target {
                                ... on Commit {
                                    history { totalCount }
                                }
                            }
                        }
                    }
                    pageInfo { endCursor hasNextPage }
                }
            }
        }
    "#;

    let mut repos = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let json = simple_request(
            "fetch_repos",
            query,
            json!({ "login": username, "cursor": cursor }),
        )?;
        let page = &json["data"]["user"]["repositories"];

        for node in page["nodes"].as_array().into_iter().flatten() {
            let Some(name) = node["nameWithOwner"].as_str() else { continue };
            if EXCLUDED_REPOS.iter().any(|ex| ex.eq_ignore_ascii_case(name)) {
                continue;
            }
            repos.push(Repo {
                name_with_owner: name.to_owned(),
                stars: u64_at(node, "/stargazerCount"),
                total_commits: u64_at(node, "/defaultBranchRef/target/history/totalCount"),
            });
        }

        if !page["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false) {
            return Ok(repos);
        }
        cursor = page["pageInfo"]["endCursor"].as_str().map(str::to_owned);
    }
}

/// Commits, additions, and deletions authored by `author_id` on the repo's
/// default branch. Filtering by author server-side keeps this to one or two
/// pages even for repos with huge upstream histories.
fn repo_loc(name_with_owner: &str, author_id: &str) -> Result<(u64, u64, u64)> {
    let query = r#"
        query ($owner: String!, $name: String!, $author: ID!, $cursor: String) {
            repository(owner: $owner, name: $name) {
                defaultBranchRef {
                    target {
                        ... on Commit {
                            history(first: 100, after: $cursor, author: { id: $author }) {
                                nodes { additions deletions }
                                pageInfo { endCursor hasNextPage }
                            }
                        }
                    }
                }
            }
        }
    "#;

    let (owner, name) = name_with_owner
        .split_once('/')
        .ok_or_else(|| format!("repo_loc: malformed repo name {name_with_owner}"))?;

    let (mut commits, mut added, mut deleted) = (0, 0, 0);
    let mut cursor: Option<String> = None;
    loop {
        let json = simple_request(
            "repo_loc",
            query,
            json!({ "owner": owner, "name": name, "author": author_id, "cursor": cursor }),
        )?;
        let history = &json["data"]["repository"]["defaultBranchRef"]["target"]["history"];
        if history.is_null() {
            // Empty repo: no default branch yet.
            return Ok((0, 0, 0));
        }

        for node in history["nodes"].as_array().into_iter().flatten() {
            commits += 1;
            added += u64_at(node, "/additions");
            deleted += u64_at(node, "/deletions");
        }

        if !history["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false) {
            return Ok((commits, added, deleted));
        }
        cursor = history["pageInfo"]["endCursor"].as_str().map(str::to_owned);
    }
}

fn repo_hash(name_with_owner: &str) -> String {
    hex::encode(Sha256::digest(name_with_owner.as_bytes()))
}

/// Header lines plus entries keyed by repo hash. Malformed lines are dropped,
/// so their repos simply get recomputed.
fn read_cache(path: &Path) -> Result<(Vec<String>, HashMap<String, CacheEntry>)> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err.into()),
    };

    let mut lines = content.lines();
    let mut comments: Vec<String> = lines.by_ref().take(COMMENT_SIZE).map(str::to_owned).collect();
    comments.resize(
        COMMENT_SIZE,
        "This line is a comment block. Write whatever you want here.".to_owned(),
    );

    let entries = lines
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let [hash, total, mine, added, deleted] = parts[..] else { return None };
            if hash.len() != 64 {
                return None;
            }
            Some((
                hash.to_owned(),
                CacheEntry {
                    total_commits: total.parse().ok()?,
                    my_commits: mine.parse().ok()?,
                    added: added.parse().ok()?,
                    deleted: deleted.parse().ok()?,
                },
            ))
        })
        .collect();

    Ok((comments, entries))
}

fn write_cache(
    path: &Path,
    comments: &[String],
    repos: &[Repo],
    entries: &HashMap<String, CacheEntry>,
) -> Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    for line in comments {
        writeln!(file, "{line}")?;
    }
    for repo in repos {
        let hash = repo_hash(&repo.name_with_owner);
        if let Some(e) = entries.get(&hash) {
            writeln!(
                file,
                "{hash} {} {} {} {}",
                e.total_commits, e.my_commits, e.added, e.deleted
            )?;
        }
    }
    file.flush()?;
    Ok(())
}

/// LOC and commit totals across `repos`, recomputing only repos whose commit
/// count changed since the cached run.
pub fn loc_stats(
    repos: &[Repo],
    username: &str,
    author_id: &str,
    force_refresh: bool,
) -> Result<LocStats> {
    fs::create_dir_all("cache")?;
    let path = Path::new("cache").join(format!("{}.txt", repo_hash(username)));
    let (comments, mut entries) = read_cache(&path)?;

    let mut refreshed = 0;
    for repo in repos {
        let hash = repo_hash(&repo.name_with_owner);
        let up_to_date = !force_refresh
            && entries
                .get(&hash)
                .is_some_and(|e| e.total_commits == repo.total_commits);
        if up_to_date {
            continue;
        }

        let (my_commits, added, deleted) = repo_loc(&repo.name_with_owner, author_id)?;
        entries.insert(
            hash,
            CacheEntry { total_commits: repo.total_commits, my_commits, added, deleted },
        );
        refreshed += 1;
        // Persist per repo so an API failure later in the run keeps this work.
        write_cache(&path, &comments, repos, &entries)?;
    }
    // Also drops entries for repos that disappeared or were excluded.
    write_cache(&path, &comments, repos, &entries)?;

    let mut stats = LocStats { added: 0, deleted: 0, commits: 0, refreshed };
    for repo in repos {
        if let Some(e) = entries.get(&repo_hash(&repo.name_with_owner)) {
            stats.added += e.added;
            stats.deleted += e.deleted;
            stats.commits += e.my_commits;
        }
    }
    Ok(stats)
}

/// Total (issues, pull requests) opened by the user.
pub fn stats_getter(username: &str) -> Result<(u64, u64)> {
    let query = r#"
        query($login: String!) {
            user(login: $login) {
                issues { totalCount }
                pullRequests { totalCount }
            }
        }
    "#;
    let json = simple_request("stats_getter", query, json!({ "login": username }))?;
    let user = &json["data"]["user"];
    Ok((
        u64_at(user, "/issues/totalCount"),
        u64_at(user, "/pullRequests/totalCount"),
    ))
}

/// Replace the text of each `<tspan id=…>` named in `values`.
pub fn svg_overwrite(filename: &str, values: &[(&str, String)]) -> Result<()> {
    let mut root = Element::parse(fs::read(filename)?.as_slice())?;
    for (id, text) in values {
        let element = find_by_id(&mut root, id)
            .ok_or_else(|| format!("{filename}: no element with id=\"{id}\""))?;
        element.children = vec![XMLNode::Text(text.clone())];
    }
    root.write(BufWriter::new(File::create(filename)?))?;
    Ok(())
}

fn find_by_id<'a>(element: &'a mut Element, id: &str) -> Option<&'a mut Element> {
    if element.attributes.get("id").is_some_and(|v| v == id) {
        return Some(element);
    }
    element.children.iter_mut().find_map(|child| match child {
        XMLNode::Element(e) => find_by_id(e, id),
        _ => None,
    })
}
