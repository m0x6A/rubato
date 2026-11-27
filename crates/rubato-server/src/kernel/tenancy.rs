//! Who a request or a poll pass acts for (ADR 0087, design §4.2). This pull request brings the
//! tenant's id, which the GitHub registry routes by; the scope types that only a membership
//! check can make come with the routes that need them.

use rubato_protocol::is_tenant_id;

/// The tenant the seed makes from a server as it was before tenants (design §8). A server
/// reading with a personal token or local repositories has one GitHub context and so one
/// tenant: this one.
pub const DEFAULT_TENANT: &str = "default";

/// A well-formed tenant id. Holding one says nothing about who may act for it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TenantId(String);

impl TenantId {
    /// `None` unless `s` is in the tenant alphabet (`[a-z0-9-]{1,32}`).
    pub fn parse(s: &str) -> Option<TenantId> {
        is_tenant_id(s).then(|| TenantId(s.to_owned()))
    }

    pub fn default_tenant() -> TenantId {
        TenantId(DEFAULT_TENANT.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_tenant_alphabet_parses() {
        assert_eq!(TenantId::parse("acme").unwrap().as_str(), "acme");
        assert!(TenantId::parse("Acme").is_none());
        assert!(TenantId::parse("").is_none());
        assert_eq!(TenantId::default_tenant().as_str(), DEFAULT_TENANT);
    }
}
