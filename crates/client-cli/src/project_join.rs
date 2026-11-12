//! Joining a checkout to an existing project by its git remote, for
//! `apas register`.
//!
//! A project can run on several machines, one instance on each. On a home
//! shared between machines they all see the same checkout, but a separate
//! checkout of the same repository, a laptop's say, used to become an
//! unrelated project the moment it was registered. Registration now asks the
//! server which of the user's projects use the checkout's repository, and
//! offers to join one, so the checkout becomes another machine of that
//! project rather than a project of its own.
//!
//! Joining replaces the folder's own identity, so it is only ever done to a
//! pristine one: no tabs in `.apas` and no session on the server. A project
//! with a session has history that a new identity would orphan, even with no
//! tabs left, and only the server can say whether there is one
//! ([`ProjectsByRemoteResponse::local`]). Without its answer nothing is
//! joined.

use crate::project::FolderIdentity;
use anyhow::{Context, Result};
use shared::{ProjectRemoteMatch, ProjectsByRemoteResponse};
use std::collections::HashSet;
use std::time::Duration;
use uuid::Uuid;

/// Registration waits this long for the server at most, connection included:
/// an unreachable server means registering the folder as it is, not hanging.
const LOOKUP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(15);

/// `apas register`'s say in joining.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JoinOptions {
    /// `--project <id>`: join this project, without asking.
    pub project: Option<Uuid>,
    /// `--new`: keep the folder a project of its own, without asking anyone.
    pub new: bool,
    /// `--yes`: join the one matching project without asking.
    pub yes: bool,
}

/// What registration asks the server: the folder's canonical remote, and the
/// identity the folder registers under now, so the server can say whether
/// that identity has history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupQuery {
    pub git_remote: String,
    pub project_id: Option<Uuid>,
}

/// What registration does with the folder.
#[derive(Debug, Clone)]
pub enum Decision {
    /// Register it under its own identity, as registration always has.
    /// `notes` say why it was not joined, where someone would wonder.
    AsIs { notes: Vec<String> },
    /// Give it this project's identity.
    Join { project: ProjectRemoteMatch },
}

/// Decide whether `folder`, a checkout of `git_remote`, joins one of the
/// user's existing projects or is registered under its own identity.
///
/// `lookup` asks the server, and is asked only when a join is possible at
/// all: the folder's project has no tabs and the folder has a remote. A
/// failed lookup registers the folder as it is, with a note, unless
/// `--project` asked for a join, which then fails and changes nothing.
/// `confirm` puts the join question to the person at the terminal, and is
/// asked only when `interactive`.
///
/// Errors when the user has to choose: several projects match, or the join
/// `--project` asked for cannot happen.
pub fn decide(
    folder: &FolderIdentity,
    git_remote: Option<&str>,
    options: &JoinOptions,
    interactive: bool,
    lookup: &mut dyn FnMut(&LookupQuery) -> Result<ProjectsByRemoteResponse>,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> Result<Decision> {
    let as_is = |notes: Vec<String>| Ok(Decision::AsIs { notes });
    if options.new {
        return as_is(Vec::new());
    }
    let own = folder.project_id();
    let explicit = options.project.filter(|project| Some(*project) != own);
    if options.project.is_some() && explicit.is_none() {
        // `--project` naming the folder's own project: nothing to join.
        return as_is(Vec::new());
    }
    let own_label = own.map_or_else(
        || "this folder's project".to_string(),
        |id| format!("project {id}"),
    );

    let tabs = folder.pane_count();
    if tabs > 0 {
        if let Some(target) = explicit {
            anyhow::bail!(
                "Not joining {target}: this folder is {own_label}, which has {tabs} tab(s). \
                 Joining would orphan them; only a project with no tabs and no history can join \
                 another"
            );
        }
        return as_is(Vec::new());
    }
    let Some(git_remote) = git_remote else {
        if let Some(target) = explicit {
            anyhow::bail!(
                "Not joining {target}: this folder has no git `origin`, so it cannot be matched \
                 to that project's repository"
            );
        }
        return as_is(Vec::new());
    };

    let answer = match lookup(&LookupQuery {
        git_remote: git_remote.to_string(),
        project_id: own,
    }) {
        Ok(answer) => answer,
        Err(error) => {
            if let Some(target) = explicit {
                anyhow::bail!(
                    "Not joining {target}: could not ask the server about it ({error:#}). \
                     Nothing was changed"
                );
            }
            return as_is(vec![format!(
                "Could not look up your projects for {git_remote} ({error:#}), so this folder \
                 keeps its own identity. To join one later, run `apas register` here again \
                 before it has tabs or has been started."
            )]);
        }
    };

    // History on the server. `local` answers for the folder's own identity
    // whatever remote that project recorded; a match for it says the same.
    let has_history = own.is_some_and(|own| {
        answer.local.as_ref().is_some_and(|local| local.has_session)
            || answer
                .projects
                .iter()
                .any(|project| project.project_id == own && project.has_session)
    });
    let mut seen = HashSet::new();
    let candidates: Vec<ProjectRemoteMatch> = answer
        .projects
        .into_iter()
        .filter(|project| Some(project.project_id) != own && seen.insert(project.project_id))
        .collect();
    if has_history {
        if let Some(target) = explicit {
            anyhow::bail!(
                "Not joining {target}: this folder is {own_label}, which has history on the \
                 server. A new identity would orphan it"
            );
        }
        let notes = if candidates.is_empty() {
            Vec::new()
        } else {
            vec![format!(
                "This folder is {own_label}, which has history, so it keeps its identity \
                 rather than joining another project that uses {git_remote}."
            )]
        };
        return as_is(notes);
    }

    if let Some(target) = explicit {
        let Some(project) = candidates
            .iter()
            .find(|project| project.project_id == target)
        else {
            let matching = if candidates.is_empty() {
                String::new()
            } else {
                format!(" Those that do:{}", listing(&candidates))
            };
            anyhow::bail!(
                "Not joining {target}: it is not one of your projects that use {git_remote}. \
                 A project whose sessions recorded another remote does not count.{matching}"
            );
        };
        if let Some(entry) = folder.registered_elsewhere(target) {
            anyhow::bail!(
                "Not joining {}: it is already registered on this machine at {}. A home shared \
                 between machines already sees that checkout, so start the project there \
                 instead",
                describe(project),
                entry.path
            );
        }
        return Ok(Decision::Join {
            project: project.clone(),
        });
    }

    // A project this machine already has elsewhere cannot take a second
    // folder here, so it is not offered.
    let mut notes = Vec::new();
    let mut joinable = Vec::new();
    for project in candidates {
        match folder.registered_elsewhere(project.project_id) {
            Some(entry) => notes.push(format!(
                "Not joining {}: it is already registered on this machine at {}, which a home \
                 shared between machines already sees. Start it there; this folder is \
                 registered as a separate project.",
                describe(&project),
                entry.path
            )),
            None => joinable.push(project),
        }
    }
    if joinable.len() > 1 {
        anyhow::bail!(
            "{} of your projects use {git_remote}:{}\nRun `apas register --project <id>` to \
             join one of them, or `apas register --new` to register this folder as a separate \
             project",
            joinable.len(),
            listing(&joinable)
        );
    }
    let Some(project) = joinable.pop() else {
        return as_is(notes);
    };
    if options.yes {
        return Ok(Decision::Join { project });
    }
    if !interactive {
        notes.push(format!(
            "Not joining {}: nobody is at a terminal to confirm it, so this folder is \
             registered as a separate project. Run `apas register --yes` here to join it \
             instead.",
            describe(&project)
        ));
        return as_is(notes);
    }
    if confirm(&format!("Join {}? [Y/n]", describe(&project))) {
        Ok(Decision::Join { project })
    } else {
        as_is(notes)
    }
}

/// `<name> (<id>, on <host>:<path>)`, naming a checkout that is running when
/// there is one.
pub fn describe(project: &ProjectRemoteMatch) -> String {
    let name = project
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("an unnamed project");
    let checkout = project
        .instances
        .iter()
        .find(|checkout| checkout.running)
        .or_else(|| project.instances.first());
    match checkout {
        Some(checkout) => format!(
            "{name} ({}, on {}:{})",
            project.project_id, checkout.hostname, checkout.path
        ),
        None => format!("{name} ({})", project.project_id),
    }
}

fn listing(projects: &[ProjectRemoteMatch]) -> String {
    projects
        .iter()
        .map(|project| format!("\n  {}", describe(project)))
        .collect()
}

/// Put `question`, which ends in `[Y/n]`, to the person at the terminal.
/// Enter alone means yes; anything but y or yes, end of input included, means
/// no.
pub fn confirm_on_terminal(question: &str) -> bool {
    use std::io::{BufRead, Write};
    eprint!("{question} ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    match std::io::stdin().lock().read_line(&mut answer) {
        Ok(0) | Err(_) => {
            eprintln!();
            false
        }
        Ok(_) => matches!(
            answer.trim().to_ascii_lowercase().as_str(),
            "" | "y" | "yes"
        ),
    }
}

/// The client [`lookup_by_remote`] is meant to use: bounded, so registering
/// never hangs on a server that does not answer.
pub fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(LOOKUP_CONNECT_TIMEOUT)
        .timeout(LOOKUP_TIMEOUT)
        .build()
        .context("Build an HTTP client")
}

/// The server's HTTP base for the WebSocket URL the CLI is configured with,
/// derived the way `auth::login` derives it.
fn http_base(server_url: &str) -> String {
    let base = if let Some(rest) = server_url.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = server_url.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        server_url.to_string()
    };
    base.trim_end_matches('/').to_string()
}

/// `GET /projects/by-remote`: the caller's projects that use
/// `query.git_remote`, and what the server knows of `query.project_id`.
///
/// Anything but a successful JSON answer is an error, which registration
/// treats as having no answer, never as having no matches: a server from
/// before this route answers 404, and a proxy that does not route it answers
/// with the web app's HTML.
pub async fn lookup_by_remote(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    query: &LookupQuery,
) -> Result<ProjectsByRemoteResponse> {
    let url = format!("{}/projects/by-remote", http_base(server_url));
    let mut params = vec![("git_remote", query.git_remote.clone())];
    if let Some(project_id) = query.project_id {
        params.push(("project_id", project_id.to_string()));
    }
    let response = client
        .get(&url)
        .bearer_auth(token)
        .query(&params)
        .send()
        .await
        .with_context(|| format!("Reach {url}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("{url} answered {status}; the server predates joining checkouts");
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!("{url} answered {status}; run `apas login` and try again");
    }
    anyhow::ensure!(status.is_success(), "{url} answered {status}");
    let body = response
        .bytes()
        .await
        .with_context(|| format!("Read the answer from {url}"))?;
    serde_json::from_slice(&body)
        .with_context(|| format!("{url} did not answer with the projects for a repository"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// Serve one request on a loopback port with `response`, and return the
    /// request's head as received.
    fn serve_once(response: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = format!("ws://{}", listener.local_addr().expect("address"));
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut head = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).expect("read") == 0 || line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut stream = stream;
            stream.write_all(response.as_bytes()).expect("write");
            head
        });
        (address, server)
    }

    fn http(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn client() -> reqwest::Client {
        // A proxy from the environment must not take the loopback request.
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client")
    }

    fn lookup(server: &str, query: &LookupQuery) -> Result<ProjectsByRemoteResponse> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(lookup_by_remote(&client(), server, "test-token", query))
    }

    #[test]
    fn the_lookup_sends_the_remote_the_folders_id_and_the_account_token() {
        let local = Uuid::new_v4();
        let other = Uuid::new_v4();
        let body = serde_json::json!({
            "git_remote": "github.com/owner/repo",
            "projects": [{
                "project_id": other,
                "name": "repo",
                "has_session": true,
                "instances": [{ "hostname": "zoo-005", "path": "/home/u/repo", "running": true }],
            }],
            "local": { "project_id": local, "has_session": false },
        })
        .to_string();
        let (server, request) = serve_once(http("200 OK", "application/json", &body));

        let answer = lookup(
            &server,
            &LookupQuery {
                git_remote: "github.com/owner/repo".to_string(),
                project_id: Some(local),
            },
        )
        .expect("answer");
        let head = request.join().expect("server");

        let request_line = head.lines().next().expect("request line");
        assert_eq!(
            request_line,
            format!(
                "GET /projects/by-remote?git_remote=github.com%2Fowner%2Frepo&project_id={local} \
                 HTTP/1.1"
            )
        );
        assert!(
            head.lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer test-token")),
            "the account token rides as a bearer token"
        );
        assert_eq!(answer.projects.len(), 1);
        assert_eq!(answer.projects[0].project_id, other);
        assert_eq!(answer.projects[0].instances[0].hostname, "zoo-005");
        assert_eq!(answer.local.map(|local| local.project_id), Some(local));
    }

    #[test]
    fn a_folder_without_an_identity_sends_no_project_id() {
        let body = r#"{"git_remote":"github.com/owner/repo","projects":[]}"#;
        let (server, request) = serve_once(http("200 OK", "application/json", body));
        let answer = lookup(
            &server,
            &LookupQuery {
                git_remote: "github.com/owner/repo".to_string(),
                project_id: None,
            },
        )
        .expect("answer");
        let head = request.join().expect("server");
        assert!(head
            .starts_with("GET /projects/by-remote?git_remote=github.com%2Fowner%2Frepo HTTP/1.1"));
        assert!(answer.projects.is_empty() && answer.local.is_none());
    }

    /// Neither a server from before the route nor a proxy answering with the
    /// web app is an answer with no matches.
    #[test]
    fn anything_but_the_routes_json_is_no_answer() {
        let query = LookupQuery {
            git_remote: "github.com/owner/repo".to_string(),
            project_id: None,
        };
        for (response, expected) in [
            (http("404 Not Found", "text/plain", ""), "predates"),
            (
                http("200 OK", "text/html", "<!DOCTYPE html><html></html>"),
                "did not answer",
            ),
            (http("401 Unauthorized", "text/plain", ""), "apas login"),
            (http("500 Internal Server Error", "text/plain", ""), "500"),
        ] {
            let (server, request) = serve_once(response);
            let error = lookup(&server, &query).expect_err("no answer");
            request.join().expect("server");
            assert!(
                format!("{error:#}").contains(expected),
                "{expected}: {error:#}"
            );
        }
    }

    #[test]
    fn an_unreachable_server_is_an_error() {
        // A port nothing listens on: bound, then released.
        let address = TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("address");
        let error = lookup(
            &format!("ws://{address}"),
            &LookupQuery {
                git_remote: "github.com/owner/repo".to_string(),
                project_id: None,
            },
        )
        .expect_err("unreachable");
        assert!(
            format!("{error:#}").contains("/projects/by-remote"),
            "{error:#}"
        );
    }

    #[test]
    fn the_http_base_follows_the_websocket_url() {
        assert_eq!(
            http_base("wss://apas.example.org"),
            "https://apas.example.org"
        );
        assert_eq!(http_base("ws://127.0.0.1:8080/"), "http://127.0.0.1:8080");
        assert_eq!(
            http_base("https://apas.example.org"),
            "https://apas.example.org"
        );
    }

    #[test]
    fn a_match_is_described_by_a_running_checkout_when_it_has_one() {
        let id = Uuid::new_v4();
        let checkout = |hostname: &str, running: bool| shared::ProjectRemoteInstance {
            machine_id: None,
            hostname: hostname.to_string(),
            path: format!("/home/u/{hostname}"),
            running,
        };
        let mut project = ProjectRemoteMatch {
            project_id: id,
            name: Some("demo".to_string()),
            has_session: true,
            instances: vec![checkout("zoo-005", false), checkout("zoo-006", true)],
        };
        assert_eq!(
            describe(&project),
            format!("demo ({id}, on zoo-006:/home/u/zoo-006)")
        );
        project.instances.clear();
        project.name = None;
        assert_eq!(describe(&project), format!("an unnamed project ({id})"));
    }
}
