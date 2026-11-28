//! One GitHub client per installation of the App, routed by tenant (ADR 0087, design §5.1).
//!
//! An installation is one GitHub context (a personal account or an organisation) and belongs to
//! at most one tenant; a repository is covered by at most one installation. So a repository has
//! at most one live tenant, and [`GitHubApps::for_repo`] answers a client only when the tenant
//! the caller acts for is that one. A repository no attached installation covers gets no client,
//! so no call is made for it.
//!
//! A server with a personal token, local repositories or the in-memory fake has one GitHub
//! context and no installation to route by: [`GitHubApps::one`]. It holds one tenant,
//! [`DEFAULT_TENANT`](crate::kernel::tenancy::DEFAULT_TENANT), and every repository is that
//! tenant's.
//!
//! The routes are the tenancy rows (`tenant_installations`, `tenant_repos`), which the kernel
//! may not read: the `tenants` slice loads them with [`GitHubApps::set_routes`] at start and
//! after every change.

use super::GitHubApi;
use crate::kernel::tenancy::TenantId;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

pub type Client = Arc<dyn GitHubApi>;

/// Makes the client of one installation. Built once per installation and kept.
pub type MakeClient = Box<dyn Fn(u64) -> anyhow::Result<Client> + Send + Sync>;

/// An installation attached to a tenant, as the routes hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub tenant: TenantId,
    /// GitHub suspended it: attached, but nothing is read through it (design §3.3).
    pub suspended: bool,
}

/// The live tenancy rows: attached installations (not detached), and the live repositories of
/// each, lower case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routes {
    pub installations: BTreeMap<u64, Attached>,
    pub repos: HashMap<String, u64>,
}

/// One installation that is read: its id (`None` for a server with one context), its tenant and
/// its client.
#[derive(Clone)]
pub struct Live {
    pub installation: Option<u64>,
    pub tenant: TenantId,
    pub client: Client,
}

enum Mode {
    One(Client),
    Installations {
        /// For calls made with a person's own token (`repo_as`, `merge_as`, `comment_as`).
        person: Client,
        make: MakeClient,
    },
}

pub struct GitHubApps {
    mode: Mode,
    routes: RwLock<Routes>,
    clients: Mutex<HashMap<u64, Client>>,
}

/// The answer for a repository no live installation covers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("repository {repo} is covered by no installation attached to a tenant")]
pub struct NotCovered {
    pub repo: String,
}

impl GitHubApps {
    /// One GitHub context: a personal token, local repositories or the fake.
    pub fn one(client: Client) -> Self {
        Self {
            mode: Mode::One(client),
            routes: RwLock::new(Routes::default()),
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// The App, with one client per attached installation, made by `make` on first use.
    pub fn installations(person: Client, make: MakeClient) -> Self {
        Self {
            mode: Mode::Installations { person, make },
            routes: RwLock::new(Routes::default()),
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// Acting as App installations, or as one installation-like port in tests.
    pub fn is_installation(&self) -> bool {
        match &self.mode {
            Mode::One(c) => c.is_installation(),
            Mode::Installations { .. } => true,
        }
    }

    /// Whether this server has per-installation routes. With one context there is nothing to
    /// route by and every repository is the one tenant's.
    pub fn routes_by_installation(&self) -> bool {
        matches!(self.mode, Mode::Installations { .. })
    }

    /// No person's token is needed to act as them: the local stand-in and the fake.
    pub fn acts_without_token(&self) -> bool {
        self.as_person().acts_without_token()
    }

    /// The client for calls made with a person's own token.
    pub fn as_person(&self) -> Client {
        match &self.mode {
            Mode::One(c) => c.clone(),
            Mode::Installations { person, .. } => person.clone(),
        }
    }

    /// Replace the routes with the tenancy rows as they now are.
    pub fn set_routes(&self, routes: Routes) {
        *self.routes.write().unwrap() = routes;
    }

    pub fn routes(&self) -> Routes {
        self.routes.read().unwrap().clone()
    }

    /// Whether a webhook delivery naming `installation` comes from an installation attached to
    /// a tenant (design §5.4). With one context every delivery is.
    pub fn admits_delivery(&self, installation: Option<u64>) -> bool {
        match &self.mode {
            Mode::One(_) => true,
            Mode::Installations { .. } => installation
                .is_some_and(|i| self.routes.read().unwrap().installations.contains_key(&i)),
        }
    }

    /// The tenant a repository is live in, and the installation that covers it.
    fn route(&self, repo: &str) -> Option<(TenantId, Option<u64>)> {
        match &self.mode {
            Mode::One(_) => Some((TenantId::default_tenant(), None)),
            Mode::Installations { .. } => {
                let r = self.routes.read().unwrap();
                let id = *r.repos.get(&repo.to_ascii_lowercase())?;
                let a = r.installations.get(&id)?;
                (!a.suspended).then(|| (a.tenant.clone(), Some(id)))
            }
        }
    }

    /// The tenant `repo` is live in: covered by an attached, not suspended installation.
    pub fn tenant_of(&self, repo: &str) -> Option<TenantId> {
        self.route(repo).map(|(t, _)| t)
    }

    /// The client for `repo` when acting for `tenant`; `None` when the repository is not live in
    /// that tenant, whatever other tenant it may be live in.
    pub fn for_repo(&self, tenant: &TenantId, repo: &str) -> Option<Client> {
        let (t, id) = self.route(repo)?;
        if &t != tenant {
            return None;
        }
        match (&self.mode, id) {
            (Mode::One(c), _) => Some(c.clone()),
            (Mode::Installations { .. }, Some(id)) => self.client(id).ok(),
            (Mode::Installations { .. }, None) => None,
        }
    }

    /// The client for `repo` when acting for `tenant`, or one whose every call fails with
    /// [`NotCovered`] when the repository is not live in that tenant: a repository outside the
    /// tenant is an error, never "absent", and never a call made for another tenant.
    pub fn within(&self, tenant: &TenantId, repo: &str) -> Client {
        self.for_repo(tenant, repo).unwrap_or_else(|| {
            Arc::new(Uncovered(NotCovered {
                repo: repo.to_owned(),
            }))
        })
    }

    /// The client of the tenant `repo` is live in, for work that acts for no person and starts
    /// from a repository: the poll's shared listings and the context graph's builds. Never for
    /// a request: a request acts for the tenant of its scope ([`Self::within`]). The files that
    /// may call this are listed in `SYSTEM_CALLERS`, and a test keeps the list exact.
    pub fn for_system(&self, repo: &str) -> Client {
        match self.tenant_of(repo) {
            Some(t) => self.within(&t, repo),
            None => Arc::new(Uncovered(NotCovered {
                repo: repo.to_owned(),
            })),
        }
    }

    /// The client of one installation, made on first use.
    pub fn client(&self, installation: u64) -> anyhow::Result<Client> {
        let Mode::Installations { make, .. } = &self.mode else {
            anyhow::bail!("this server acts as no App installation");
        };
        let mut clients = self.clients.lock().unwrap();
        if let Some(c) = clients.get(&installation) {
            return Ok(c.clone());
        }
        let c = make(installation)?;
        clients.insert(installation, c.clone());
        Ok(c)
    }

    /// Every installation that is read: attached and not suspended, in id order. With one
    /// context, that context.
    pub fn live(&self) -> Vec<Live> {
        match &self.mode {
            Mode::One(c) => vec![Live {
                installation: None,
                tenant: TenantId::default_tenant(),
                client: c.clone(),
            }],
            Mode::Installations { .. } => {
                let attached: Vec<(u64, TenantId)> = self
                    .routes
                    .read()
                    .unwrap()
                    .installations
                    .iter()
                    .filter(|(_, a)| !a.suspended)
                    .map(|(id, a)| (*id, a.tenant.clone()))
                    .collect();
                attached
                    .into_iter()
                    .filter_map(|(id, tenant)| match self.client(id) {
                        Ok(client) => Some(Live {
                            installation: Some(id),
                            tenant,
                            client,
                        }),
                        Err(e) => {
                            tracing::error!(rubato.github.installation = id, error = %format!("{e:#}"), "no client for the installation");
                            None
                        }
                    })
                    .collect()
            }
        }
    }

    /// `<app_slug>[bot]`, as the first installation read names it. The same App for all.
    pub fn app_login(&self) -> Option<String> {
        self.live().iter().find_map(|l| l.client.app_login())
    }

    /// Whether some live installation may create issues (ADR 0025). The overview shows one
    /// answer until it is built per tenant (plan 345, PR 5).
    pub fn can_create_issues_anywhere(&self) -> bool {
        self.live().iter().any(|l| l.client.can_create_issues())
    }
}

/// The client of a repository no live installation covers: every call fails, none is made.
struct Uncovered(NotCovered);

impl Uncovered {
    fn no<T>(&self) -> anyhow::Result<T> {
        Err(self.0.clone().into())
    }
}

#[async_trait::async_trait]
impl GitHubApi for Uncovered {
    async fn file(&self, _: &str, _: &str, _: &str) -> anyhow::Result<Option<String>> {
        self.no()
    }
    async fn branch_sha(&self, _: &str, _: &str) -> anyhow::Result<Option<String>> {
        self.no()
    }
    async fn last_commit_author(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Option<String>> {
        self.no()
    }
    async fn branches(&self, _: &str, _: &str) -> anyhow::Result<Vec<super::BranchHead>> {
        self.no()
    }
    async fn open_pulls(&self, _: &str) -> anyhow::Result<super::OpenPulls> {
        self.no()
    }
    async fn issue(&self, _: &str, _: u64) -> anyhow::Result<Option<super::IssueInfo>> {
        self.no()
    }
    async fn issue_comments(&self, _: &str, _: u64) -> anyhow::Result<Vec<super::Comment>> {
        self.no()
    }
    async fn pull_for_branch(&self, _: &str, _: &str) -> anyhow::Result<Option<super::PullInfo>> {
        self.no()
    }
    async fn checks(&self, _: &str, _: &str) -> anyhow::Result<rubato_protocol::ChecksStatus> {
        self.no()
    }
    async fn default_head(&self, _: &str) -> anyhow::Result<Option<String>> {
        self.no()
    }
    async fn tarball(&self, _: &str, _: &str, _: u64) -> anyhow::Result<super::Tarball> {
        self.no()
    }
    async fn pull(&self, _: &str, _: u64) -> anyhow::Result<Option<super::PullFull>> {
        self.no()
    }
    async fn pull_reviews(&self, _: &str, _: u64) -> anyhow::Result<Vec<super::ReviewInfo>> {
        self.no()
    }
    async fn pull_files(
        &self,
        _: &str,
        _: u64,
    ) -> anyhow::Result<Vec<rubato_protocol::pulls::ChangedFile>> {
        self.no()
    }
    async fn check_runs(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<Vec<rubato_protocol::pulls::CheckRun>> {
        self.no()
    }
    async fn rate_limit(&self) -> anyhow::Result<(u64, u64)> {
        self.no()
    }
    async fn open_issues(&self, _: &str, _: usize) -> anyhow::Result<super::OpenIssues> {
        self.no()
    }
    async fn create_ref(&self, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
        self.no()
    }
    async fn put_file(&self, _: &str, _: &str, _: &str, _: &str, _: &str) -> anyhow::Result<()> {
        self.no()
    }
    async fn create_pull(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> anyhow::Result<(u64, String)> {
        self.no()
    }
    async fn close_pull(&self, _: &str, _: u64) -> anyhow::Result<()> {
        self.no()
    }
    async fn delete_ref(&self, _: &str, _: &str) -> anyhow::Result<()> {
        self.no()
    }
    async fn create_issue(&self, _: &str, _: &str, _: &str) -> anyhow::Result<(u64, String)> {
        self.no()
    }
}

/// The files that may reach GitHub from a repository alone ([`GitHubApps::for_system`]): work
/// that acts for no person. No request handler is among them, and none may join without a
/// reason here.
#[cfg(test)]
pub(crate) const SYSTEM_CALLERS: &[&str] = &[
    // The context graph builds each repository's source through its own installation.
    "context_graph/build.rs",
    // The poll's per-repository listings (branches, open pull requests), shared by its issues.
    "github_poll/poll.rs",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::github::FakeGitHub;

    /// Every `.for_system(` call outside this file, by file, relative to `src/`.
    fn callers(root: &std::path::Path) -> std::collections::BTreeMap<String, usize> {
        fn walk(
            dir: &std::path::Path,
            root: &std::path::Path,
            out: &mut std::collections::BTreeMap<String, usize>,
        ) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    let rel = p
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    if rel == "kernel/github/apps.rs" {
                        continue;
                    }
                    let n = std::fs::read_to_string(&p)
                        .unwrap()
                        .matches(".for_system(")
                        .count();
                    if n > 0 {
                        out.insert(rel, n);
                    }
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    /// The list of [`SYSTEM_CALLERS`] is exact: a new caller fails, and so does a listed file
    /// that no longer calls it.
    fn unlisted_and_stale(
        found: &std::collections::BTreeMap<String, usize>,
    ) -> (Vec<String>, Vec<String>) {
        let unlisted = found
            .keys()
            .filter(|f| !SYSTEM_CALLERS.contains(&f.as_str()))
            .cloned()
            .collect();
        let stale = SYSTEM_CALLERS
            .iter()
            .filter(|f| !found.contains_key(**f))
            .map(|f| (*f).to_owned())
            .collect();
        (unlisted, stale)
    }

    #[test]
    fn only_the_listed_files_reach_github_without_a_tenant() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let found = callers(&root);
        let (unlisted, stale) = unlisted_and_stale(&found);
        assert!(
            unlisted.is_empty(),
            "new callers of GitHubApps::for_system: {unlisted:?}; a request uses within() with its tenant"
        );
        assert!(
            stale.is_empty(),
            "no longer call for_system, take them off SYSTEM_CALLERS: {stale:?}"
        );
        println!(
            "checked {} for_system calls in {} files",
            found.values().sum::<usize>(),
            found.len()
        );
    }

    #[test]
    fn must_fail_a_new_caller_and_a_stale_entry_are_reported() {
        let mut found: std::collections::BTreeMap<String, usize> = SYSTEM_CALLERS
            .iter()
            .map(|f| ((*f).to_owned(), 1))
            .collect();
        found.insert("pins/handlers.rs".into(), 1);
        found.remove("github_poll/poll.rs");
        assert_eq!(
            unlisted_and_stale(&found),
            (
                vec!["pins/handlers.rs".to_owned()],
                vec!["github_poll/poll.rs".to_owned()]
            )
        );
    }

    fn t(s: &str) -> TenantId {
        TenantId::parse(s).unwrap()
    }

    /// Two installations: 11 in `acme` covering `acme/api`, 22 in `bcorp` covering `bcorp/web`.
    fn two() -> (GitHubApps, Arc<FakeGitHub>, Arc<FakeGitHub>) {
        let (a, b) = (Arc::new(FakeGitHub::new()), Arc::new(FakeGitHub::new()));
        a.with(|g| g.installation = Some(vec!["acme/api".into()]));
        b.with(|g| g.installation = Some(vec!["bcorp/web".into()]));
        let fakes: HashMap<u64, Client> = [(11, a.clone() as Client), (22, b.clone() as Client)]
            .into_iter()
            .collect();
        let apps = GitHubApps::installations(
            Arc::new(FakeGitHub::new()),
            Box::new(move |id| {
                fakes
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no installation {id}"))
            }),
        );
        apps.set_routes(Routes {
            installations: [
                (
                    11,
                    Attached {
                        tenant: t("acme"),
                        suspended: false,
                    },
                ),
                (
                    22,
                    Attached {
                        tenant: t("bcorp"),
                        suspended: false,
                    },
                ),
            ]
            .into(),
            repos: [("acme/api".into(), 11), ("bcorp/web".into(), 22)].into(),
        });
        (apps, a, b)
    }

    fn is(c: &Option<Client>, fake: &Arc<FakeGitHub>) -> bool {
        c.as_ref().is_some_and(|c| {
            std::ptr::addr_eq(Arc::as_ptr(c), Arc::as_ptr(fake) as *const dyn GitHubApi)
        })
    }

    #[tokio::test]
    async fn a_repo_is_reached_only_through_its_own_tenants_installation() {
        let (apps, a, b) = two();
        assert!(is(&apps.for_repo(&t("acme"), "Acme/API"), &a));
        assert!(is(&apps.for_repo(&t("bcorp"), "bcorp/web"), &b));
        assert!(
            apps.for_repo(&t("acme"), "bcorp/web").is_none(),
            "another tenant's repository gets no client"
        );
        assert!(apps.for_repo(&t("acme"), "stranger/repo").is_none());
        assert_eq!(apps.tenant_of("bcorp/web"), Some(t("bcorp")));
        let err = apps
            .for_system("stranger/repo")
            .file("stranger/repo", "x", "main")
            .await
            .unwrap_err();
        assert_eq!(
            err.downcast_ref::<NotCovered>(),
            Some(&NotCovered {
                repo: "stranger/repo".into()
            }),
            "an uncovered repository is an error, not an absent file"
        );
        assert_eq!(apps.live().len(), 2);
        println!("checked 4 repositories across 2 installations");
    }

    #[test]
    fn a_suspended_installation_is_attached_but_not_read() {
        let (apps, _, _) = two();
        let mut r = apps.routes();
        r.installations.get_mut(&22).unwrap().suspended = true;
        apps.set_routes(r);
        assert!(apps.for_repo(&t("bcorp"), "bcorp/web").is_none());
        assert_eq!(apps.tenant_of("bcorp/web"), None);
        assert!(apps.admits_delivery(Some(22)), "its unsuspend must arrive");
        assert_eq!(
            apps.live()
                .iter()
                .map(|l| l.installation)
                .collect::<Vec<_>>(),
            [Some(11)]
        );
    }

    #[test]
    fn a_delivery_from_an_unattached_installation_is_not_admitted() {
        let (apps, _, _) = two();
        assert!(apps.admits_delivery(Some(11)));
        assert!(!apps.admits_delivery(Some(99)));
        assert!(!apps.admits_delivery(None));
        let one = GitHubApps::one(Arc::new(FakeGitHub::new()));
        assert!(
            one.admits_delivery(None),
            "one context has nothing to check"
        );
    }

    #[test]
    fn one_context_is_the_default_tenant_for_every_repository() {
        let fake = Arc::new(FakeGitHub::new());
        let apps = GitHubApps::one(fake.clone());
        assert!(is(
            &apps.for_repo(&TenantId::default_tenant(), "any/repo"),
            &fake
        ));
        assert!(apps.for_repo(&t("acme"), "any/repo").is_none());
        assert!(!apps.routes_by_installation());
        assert!(apps.client(1).is_err());
    }

    #[test]
    fn an_installation_client_is_made_once() {
        let made = Arc::new(Mutex::new(0));
        let m = made.clone();
        let apps = GitHubApps::installations(
            Arc::new(FakeGitHub::new()),
            Box::new(move |_| {
                *m.lock().unwrap() += 1;
                Ok(Arc::new(FakeGitHub::new()) as Client)
            }),
        );
        apps.client(5).unwrap();
        apps.client(5).unwrap();
        apps.client(6).unwrap();
        assert_eq!(*made.lock().unwrap(), 2);
    }
}
