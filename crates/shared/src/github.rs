use std::fmt;

/// A credential-free public GitHub repository identity accepted for shared
/// cluster provisioning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalGithubRepository {
    pub owner: String,
    pub repository: String,
}

impl CanonicalGithubRepository {
    pub fn git_remote(&self) -> String {
        format!("github.com/{}/{}", self.owner, self.repository)
    }

    pub fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repository)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GithubRepositoryError;

impl fmt::Display for GithubRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("Use a public GitHub URL in the form https://github.com/owner/repository")
    }
}

impl std::error::Error for GithubRepositoryError {}

/// Parse the deliberately narrow URL form used by shared machines. This does
/// not attempt to be a general URL parser: rejecting every alternate spelling
/// avoids userinfo, ports, percent-encoding, Unicode-host, and local-path
/// ambiguities before any input reaches Git.
pub fn parse_public_github_repository(
    input: &str,
) -> Result<CanonicalGithubRepository, GithubRepositoryError> {
    if input.is_empty()
        || input != input.trim()
        || !input.is_ascii()
        || input.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(GithubRepositoryError);
    }
    let path = input
        .strip_prefix("https://github.com/")
        .ok_or(GithubRepositoryError)?;
    if path.contains(['?', '#', '@', ':', '\\', '%']) || path.ends_with('/') {
        return Err(GithubRepositoryError);
    }
    let mut components = path.split('/');
    let owner = components.next().ok_or(GithubRepositoryError)?;
    let raw_repository = components.next().ok_or(GithubRepositoryError)?;
    if components.next().is_some() {
        return Err(GithubRepositoryError);
    }
    let repository = raw_repository
        .strip_suffix(".git")
        .unwrap_or(raw_repository);
    if !valid_owner(owner) || !valid_repository(repository) {
        return Err(GithubRepositoryError);
    }
    Ok(CanonicalGithubRepository {
        owner: owner.to_string(),
        repository: repository.to_string(),
    })
}

fn valid_owner(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 39
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn valid_repository(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Errors from anonymous shared clones are intentionally generic: Git output
/// can contain proxy URLs, helper diagnostics, or repository details that
/// should not cross the cluster-owner/member boundary.
pub fn scrub_shared_clone_error(_error: &dyn std::fmt::Display) -> &'static str {
    "The public GitHub repository could not be cloned anonymously"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_canonicalizes_public_github_https() {
        for input in [
            "https://github.com/openai/codex",
            "https://github.com/openai/codex.git",
            "https://github.com/Some-Org/repo_name.git",
        ] {
            let parsed = parse_public_github_repository(input).unwrap();
            assert_eq!(parsed.clone_url().matches("https://").count(), 1);
            assert!(parsed.clone_url().ends_with(".git"));
            assert!(!parsed.git_remote().ends_with(".git"));
        }
    }

    #[test]
    fn rejects_credentials_hosts_ports_queries_fragments_and_local_syntax() {
        for input in [
            "http://github.com/openai/codex",
            "https://user:token@github.com/openai/codex",
            "https://github.com:443/openai/codex",
            "https://github.example/openai/codex",
            "https://github.com/openai/codex?tab=readme",
            "https://github.com/openai/codex#readme",
            "https://github.com/openai/codex/extra",
            "git@github.com:openai/codex.git",
            "ssh://git@github.com/openai/codex.git",
            "file:///tmp/repo",
            "../repo",
            "https://github.com/openai/%63odex",
            "https://githuв.com/openai/codex",
            "https://github.com/openai/codex/",
            " https://github.com/openai/codex",
        ] {
            assert!(
                parse_public_github_repository(input).is_err(),
                "unexpectedly accepted {input}"
            );
        }
    }

    #[test]
    fn errors_never_echo_sensitive_input() {
        let raw = "https://token@example.invalid/repository";
        let message = scrub_shared_clone_error(&raw);
        assert!(!message.contains("token"));
        assert!(!message.contains("example.invalid"));
    }
}
