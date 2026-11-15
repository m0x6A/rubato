//! GitHub behind one trait: octocrab in production, local git repos for e2e runs, an in-memory
//! fake in tests. Items are re-exported here so slices name them `kernel::github::<item>`.

mod api;
mod app;
mod fake;
mod local;

pub use api::{
    BODY_EXCERPT_BYTES, BranchHead, Comment, FILES_PER_PAGE, GitHubApi, ISSUES_PER_PAGE, IssueInfo,
    Octo, OpenIssueInfo, OpenIssues, OpenPulls, PULLS_PER_PAGE, PostError, PostedComment, PullFull,
    PullInfo, READ_TIMEOUT, RepoAccess, ReviewInfo, USER_WRITE_TIMEOUT, issues_from_page,
    summarize_check_runs,
};
pub use app::{AppCheckError, check_app};
pub use fake::{FakeGitHub, FakeInner};
pub use local::{DevPull, LocalGit};

/// Open issues per repo from a `RUBATO_DEV_ISSUES` file: `{"owner/name": [{number, title, url}]}`.
pub fn read_dev_issues(
    path: &std::path::Path,
) -> anyhow::Result<std::collections::HashMap<String, Vec<OpenIssueInfo>>> {
    use anyhow::Context;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("RUBATO_DEV_ISSUES {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("RUBATO_DEV_ISSUES {}", path.display()))
}

/// Open pull requests per repo from a `RUBATO_DEV_PULLS` file, for the local stand-in (#82).
pub fn read_dev_pulls(
    path: &std::path::Path,
) -> anyhow::Result<std::collections::HashMap<String, Vec<DevPull>>> {
    use anyhow::Context;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("RUBATO_DEV_PULLS {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("RUBATO_DEV_PULLS {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_installation_local_and_test_ports_can_write() {
        // A personal-token client reads; it does not write (ADR 0018).
        let token = Octo::new("t").unwrap();
        assert!(!token.can_write() && !token.is_installation());

        let base = app::tests::github_standin().await;
        let app = crate::kernel::config::GitHubApp {
            app_id: 12,
            private_key: app::tests::test_key(),
            installation_id: 34,
        };
        let installation = Octo::for_installation(&app, Some(&base)).unwrap();
        assert!(installation.can_write() && installation.is_installation());
        assert_eq!(
            installation.installation_repos().await.unwrap(),
            ["o/covered"],
            "the installation reads through its own token"
        );
        // ADR 0025: creating issues waits for the permission read, as the App.
        assert!(!installation.can_create_issues());
        assert_eq!(installation.app_login(), None, "not read yet");
        assert!(installation.refresh_issue_permission().await.unwrap());
        assert!(installation.can_create_issues());
        // Issue #165: the same read names the login the App's own issues carry.
        assert_eq!(
            installation.app_login().as_deref(),
            Some("rubato-test[bot]")
        );
        assert!(!token.can_create_issues());
        assert!(!token.refresh_issue_permission().await.unwrap());
        assert_eq!(token.app_login(), None);

        // The tokenless fake cannot write until a test turns writes on.
        let fake = FakeGitHub::new();
        assert!(!fake.can_write() && !fake.is_installation());
        fake.with(|g| g.writes = true);
        assert!(fake.can_write());

        let tmp = tempfile::tempdir().unwrap();
        let local = LocalGit::from_spec(&format!("o/r={}", tmp.path().display())).unwrap();
        assert!(local.can_write() && !local.is_installation());
        println!("checked 4 ports");
    }

    #[test]
    fn dev_issues_file_is_read_per_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("issues.json");
        std::fs::write(
            &path,
            r#"{"o/r": [{"number": 3, "title": "T", "url": "u", "labels": ["bug"]}]}"#,
        )
        .unwrap();
        let issues = read_dev_issues(&path).unwrap();
        assert_eq!(issues["o/r"][0].number, 3);
        assert_eq!(issues["o/r"][0].labels, ["bug"]);
        assert!(read_dev_issues(&tmp.path().join("missing.json")).is_err());
    }
}
