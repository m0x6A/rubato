//! A plan bundle from the companion becomes a revision (ADR 0053). The caller resolves the launch
//! and passes its paired user: this slice names only `plans`, so `launch` can name it (PR 8).

use super::store::{self, RevisionRow};
use crate::kernel::state::App;
use crate::plans;
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::{DraftBundle, DraftRevisionInfo, WebResource, approval_hash};
use tracing::Instrument;

fn refused(app: &App, reason: &'static str) {
    app.metrics
        .drafts_refused
        .add(1, &[KeyValue::new(attr::DROP_REASON, reason)]);
}

/// Store the bundle as the next revision of its branch's draft, owned by `owner` (the launch's
/// paired user). Another person's bundle on a draft someone else owns is not stored.
pub async fn on_draft(app: &App, bundle: &DraftBundle, owner: &str) {
    let span = tracing::info_span!(span::PLAN_DRAFT, rubato.issue = %bundle.issue, rubato.branch = %bundle.branch);
    async move {
        if let Some(r) = bundle.refused {
            refused(app, "too_large");
            tracing::warn!(rubato.issue = %bundle.issue, ?r, "draft bundle refused on the machine");
            return;
        }
        let Some(plan_file) = bundle.files.first() else {
            refused(app, "no_plan");
            return;
        };
        let now = app.clock.now();
        let draft = match store::get_or_create(&app.db, &bundle.issue, &bundle.branch, owner, now).await {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "reading the draft failed");
                return;
            }
        };
        if !draft.owner.eq_ignore_ascii_case(owner) {
            refused(app, "owner_conflict");
            tracing::warn!(rubato.issue = %bundle.issue, owner = %draft.owner, other = %owner, "draft bundle from someone else's session not stored");
            if let Err(e) = store::record_conflict(&app.db, draft.id, owner, now).await {
                tracing::error!(error = %format!("{e:#}"), "recording a draft conflict failed");
            }
            app.notify("draft conflict");
            return;
        }
        let (plan, parse_error) = match plans::parse_plan(&plan_file.text, &plan_file.path) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let rev = RevisionRow {
            info: DraftRevisionInfo {
                rev: 0,
                at: bundle.at,
                plan_sha: approval_hash(&plan_file.text),
                bundle_hash: bundle.bundle_hash.clone(),
                session_id: bundle.session_id.clone(),
                missing: bundle.missing.clone(),
                parse_error,
            },
            launch_id: Some(bundle.launch_id),
            files: bundle.files.clone(),
            plan,
        };
        match store::insert_revision(&app.db, draft.id, &rev).await {
            Ok(Some(n)) => {
                app.metrics.drafts_revisions.add(1, &[]);
                tracing::info!(rubato.issue = %bundle.issue, rubato.branch = %bundle.branch, rubato.draft.rev = n, "draft revision stored");
                app.notify_resource(WebResource::Draft { draft_id: draft.id });
                app.notify("draft");
            }
            Ok(None) => tracing::debug!(rubato.issue = %bundle.issue, "draft bundle unchanged"),
            Err(e) => tracing::error!(error = %format!("{e:#}"), "storing a draft revision failed"),
        }
    }
    .instrument(span)
    .await
}
