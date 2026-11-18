//! What the traffic adapters publish for a workspace's settings (ADR 0075): each adapter's field
//! list, and the one check of a source's fields and secrets before they are stored. Only this
//! slice knows what a field means (ADR 0040); the settings slice passes text through.

use super::guard;
#[cfg(test)]
use super::prometheus::FIELDS;
use super::prometheus::{PrometheusConfig, SECRETS, SOURCE};
use crate::kernel::config::TRAFFIC_FETCH_SECS;
use rubato_protocol::{FieldKind, SourceField, SourceFields};
use std::collections::BTreeMap;

/// Why a source's fields are refused. `url` tells a refused URL (`url_refused`) from any other
/// value that does not read. The message never carries a secret's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRefusal {
    pub url: bool,
    pub message: String,
}

impl FieldRefusal {
    fn url(message: impl Into<String>) -> Self {
        Self {
            url: true,
            message: message.into(),
        }
    }

    fn value(message: impl Into<String>) -> Self {
        Self {
            url: false,
            message: message.into(),
        }
    }
}

fn field(
    name: &str,
    label: &str,
    kind: FieldKind,
    required: bool,
    secret: bool,
    placeholder: Option<&str>,
) -> SourceField {
    SourceField {
        name: name.into(),
        label: label.into(),
        kind,
        required,
        secret,
        placeholder: placeholder.map(str::to_owned),
    }
}

/// Every adapter a workspace can choose, with its fields in form order.
pub fn sources() -> Vec<SourceFields> {
    vec![SourceFields {
        source: SOURCE.into(),
        label: "Prometheus service graph".into(),
        fields: vec![
            field("url", "Query API URL", FieldKind::Url, true, false, None),
            field(
                "headers",
                "Request headers",
                FieldKind::Text,
                false,
                true,
                Some("Name=value, Name=value"),
            ),
            field(
                "namespace_labels",
                "Namespace labels, client first",
                FieldKind::Text,
                false,
                false,
                Some("client_service_namespace,server_service_namespace"),
            ),
            field(
                "metric_prefix",
                "Metric prefix",
                FieldKind::Text,
                false,
                false,
                Some("traces_service_graph_"),
            ),
            field(
                "namespaces",
                "Service namespaces to include (empty: all)",
                FieldKind::Text,
                false,
                false,
                Some("rubato, shop"),
            ),
            field(
                "window_secs",
                "Window, seconds",
                FieldKind::Number,
                false,
                false,
                Some("3600"),
            ),
            field(
                "timeout_secs",
                "Query timeout, seconds",
                FieldKind::Number,
                false,
                false,
                Some("10"),
            ),
            field(
                "fetch_secs",
                "Seconds between fetches",
                FieldKind::Number,
                false,
                false,
                Some("60"),
            ),
        ],
    }]
}

/// The names of an adapter's secret fields; empty for an adapter this server does not have.
pub fn secret_names(source: &str) -> Vec<&'static str> {
    match source {
        SOURCE => SECRETS.iter().map(|(f, _)| *f).collect(),
        _ => Vec::new(),
    }
}

/// `scheme://host[:port]` of the source's URL, lower case: what a stored secret is bound to. A
/// change of it clears the secrets (ADR 0075).
pub fn origin(fields: &BTreeMap<String, String>) -> Option<String> {
    let u = url::Url::parse(fields.get("url")?).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    Some(match u.port_or_known_default() {
        Some(p) => format!("{}://{host}:{p}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
}

/// Check a source before it is stored: a known adapter, only its fields, a URL that is http(s)
/// with no userinfo and, unless the server is itself local, no refused address as an IP literal
/// (a name is checked at every connect, `guard`), and values the adapter reads.
pub fn check(
    source: &str,
    fields: &BTreeMap<String, String>,
    secrets: &BTreeMap<String, String>,
    server_is_local: bool,
) -> Result<(), FieldRefusal> {
    if source != SOURCE {
        return Err(FieldRefusal::value(format!(
            "{source:?} is not a traffic source this server has"
        )));
    }
    let known = sources().remove(0).fields;
    for name in fields.keys() {
        if !known.iter().any(|f| !f.secret && f.name == *name) {
            return Err(FieldRefusal::value(format!(
                "{name:?} is not a field of the {SOURCE} source"
            )));
        }
    }
    for name in secrets.keys() {
        if !known.iter().any(|f| f.secret && f.name == *name) {
            return Err(FieldRefusal::value(format!(
                "{name:?} is not a secret of the {SOURCE} source"
            )));
        }
    }
    let Some(raw) = fields.get("url") else {
        return Err(FieldRefusal::url("the source needs a URL"));
    };
    let parsed = url::Url::parse(raw).map_err(|e| FieldRefusal::url(format!("not a URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(FieldRefusal::url("the URL must be http or https"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(FieldRefusal::url(
            "the URL must not carry a user or password; use a header",
        ));
    }
    if parsed.host().is_none() {
        return Err(FieldRefusal::url("the URL has no host"));
    }
    if !server_is_local && guard::literal_refused(raw) {
        return Err(FieldRefusal::url(guard::REFUSED));
    }
    let (min, default, max) = TRAFFIC_FETCH_SECS;
    let fetch_secs = match fields.get("fetch_secs") {
        None => default,
        Some(v) => match v.parse::<u32>() {
            Ok(n) if (min..=max).contains(&n) => n,
            _ => {
                return Err(FieldRefusal::value(format!(
                    "fetch_secs must be a number from {min} to {max}"
                )));
            }
        },
    };
    // The adapter's own validator, the one the environment went through. Its messages name the
    // variable a field was seeded from and never a header's value.
    PrometheusConfig::from_fields(fields, secrets, fetch_secs)
        .map(|_| ())
        .map_err(FieldRefusal::value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn the_field_list_covers_what_the_adapter_reads() {
        let list = sources();
        assert_eq!(list.len(), 1);
        let names: Vec<&str> = list[0].fields.iter().map(|f| f.name.as_str()).collect();
        for (f, _) in FIELDS {
            assert!(names.contains(&f), "{f} is in the field list");
        }
        for (f, _) in SECRETS {
            assert!(
                list[0].fields.iter().any(|x| x.name == f && x.secret),
                "{f} is a secret in the field list"
            );
        }
        assert_eq!(secret_names(SOURCE), ["headers"]);
        println!("checked {} fields", names.len());
    }

    #[test]
    fn a_source_is_checked_before_it_is_stored() {
        let none = BTreeMap::new();
        let ok = fields(&[("url", "http://prom.lan:9090"), ("window_secs", "1800")]);
        assert_eq!(check(SOURCE, &ok, &none, false), Ok(()));
        let cases: &[(&[(&str, &str)], bool)] = &[
            (&[("url", "ftp://prom.lan")], true),
            (&[("url", "http://user:pw@prom.lan")], true),
            (&[("url", "http://169.254.169.254/")], true),
            (&[("url", "not a url")], true),
            (&[], true),
            (
                &[("url", "http://prom.lan"), ("window_secs", "soon")],
                false,
            ),
            (&[("url", "http://prom.lan"), ("fetch_secs", "1")], false),
            (&[("url", "http://prom.lan"), ("colour", "red")], false),
        ];
        for (f, url) in cases {
            let e = check(SOURCE, &fields(f), &none, false).unwrap_err();
            assert_eq!(e.url, *url, "{f:?}: {e:?}");
        }
        assert!(check("graphite", &ok, &none, false).is_err());
        // A local server may read a source on its own machine.
        assert_eq!(
            check(
                SOURCE,
                &fields(&[("url", "http://127.0.0.1:9090")]),
                &none,
                true
            ),
            Ok(())
        );
        // A header that does not read is refused without its value.
        let bad = fields(&[("headers", "Authorization Bearer s3cret")]);
        let e = check(SOURCE, &ok, &bad, false).unwrap_err();
        assert!(!e.message.contains("s3cret"), "{e:?}");
        println!("checked {} refusals", cases.len() + 2);
    }

    #[test]
    fn the_origin_is_scheme_host_and_port() {
        let o = |u: &str| origin(&fields(&[("url", u)]));
        assert_eq!(o("http://Prom.lan/api"), Some("http://prom.lan:80".into()));
        assert_eq!(o("http://prom.lan:80/x"), o("http://prom.lan"));
        assert_ne!(o("http://prom.lan"), o("https://prom.lan"));
        assert_ne!(o("http://prom.lan"), o("http://prom.lan:9090"));
        assert_ne!(o("http://prom.lan"), o("http://evil.example"));
        assert_eq!(origin(&BTreeMap::new()), None);
    }
}
