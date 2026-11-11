//! The shared draft of the system map (ADR 0018): an ordered list of operations the team has
//! made on the map in the browser, kept on the server, shown as an overlay, and proposed to the
//! map repository as one pull request. Draft ids are not map ids: they cannot be pinned, and a
//! plan naming one still has unknown ids until the proposal is merged and the map reloaded.

use crate::map::{MapNode, Seam, SystemMap};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A draft holds at most this many live operations; a write past it is refused.
pub const DRAFT_MAX_OPS: usize = 200;

/// One change to the map. An update replaces the whole part or seam; its id cannot change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MapOp {
    AddNode { node: MapNode },
    UpdateNode { node: MapNode },
    RemoveNode { id: String },
    AddSeam { seam: Seam },
    UpdateSeam { seam: Seam },
    RemoveSeam { id: String },
}

impl MapOp {
    /// The id of the part or seam the operation is about.
    pub fn target(&self) -> &str {
        match self {
            MapOp::AddNode { node } | MapOp::UpdateNode { node } => &node.id,
            MapOp::AddSeam { seam } | MapOp::UpdateSeam { seam } => &seam.id,
            MapOp::RemoveNode { id } | MapOp::RemoveSeam { id } => id,
        }
    }

    /// In words, for the pull request body and the draft list.
    pub fn summary(&self) -> String {
        match self {
            MapOp::AddNode { node } => format!("add part {}", node.id),
            MapOp::UpdateNode { node } => format!("change part {}", node.id),
            MapOp::RemoveNode { id } => format!("remove part {id}"),
            MapOp::AddSeam { seam } => {
                format!("add seam {} ({} to {})", seam.id, seam.from, seam.to)
            }
            MapOp::UpdateSeam { seam } => format!("change seam {}", seam.id),
            MapOp::RemoveSeam { id } => format!("remove seam {id}"),
        }
    }
}

/// Body of `POST /api/map/draft/ops`. `op_id` is made by the browser, so a retry of the same
/// request does not add the operation twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftOpRequest {
    pub op_id: Uuid,
    #[serde(flatten)]
    pub op: MapOp,
}

/// An operation in the draft, with who made it and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftOp {
    /// Its place in the draft; also its name in `DELETE /api/map/draft/ops/{seq}`.
    pub seq: u64,
    pub op_id: Uuid,
    #[serde(flatten)]
    pub op: MapOp,
    pub author: String,
    pub created_at: DateTime<Utc>,
    /// Why the operation no longer applies to the map as it is now (with the operations before
    /// it). It is skipped in the preview, and the draft cannot be proposed until it is removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    /// The server is writing the branch and opening the pull request, or a step failed.
    Opening,
    /// The pull request is open.
    Proposed,
    /// The pull request was merged; the draft empties once the map has been loaded again.
    Merged,
}

/// The one proposal there can be. While it exists the draft takes no operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Proposal {
    pub state: ProposalState,
    pub state_since: DateTime<Utc>,
    pub branch: String,
    pub proposed_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
}

/// The draft as every browser sees it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MapDraft {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<DraftOp>,
    /// The map with every operation that applies. Absent when the draft is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<SystemMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposal>,
    /// A discarded draft can still be brought back (until the next operation is written).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub can_restore: bool,
}

/// Answer of `POST /api/map/draft/propose`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProposeResponse {
    pub proposal: Proposal,
}

/// Route paths for the draft. It is read through the overview; there is no read route.
pub mod routes {
    pub const DRAFT_OPS: &str = "/api/map/draft/ops";
    pub const DRAFT_DISCARD: &str = "/api/map/draft/discard";
    pub const DRAFT_RESTORE: &str = "/api/map/draft/restore";
    pub const DRAFT_YAML: &str = "/api/map/draft/yaml";
    pub const DRAFT_PROPOSE: &str = "/api/map/draft/propose";
    pub const DRAFT_WITHDRAW: &str = "/api/map/draft/withdraw";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::NodeKind;

    #[test]
    fn an_operation_is_one_flat_object_on_the_wire() {
        let req = DraftOpRequest {
            op_id: Uuid::nil(),
            op: MapOp::AddNode {
                node: MapNode {
                    id: "billing".into(),
                    kind: NodeKind::Context,
                    repo: None,
                    path: None,
                    description: None,
                },
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "op_id": "00000000-0000-0000-0000-000000000000",
                "op": "add_node",
                "node": { "id": "billing", "kind": "context" }
            })
        );
        assert_eq!(serde_json::from_value::<DraftOpRequest>(json).unwrap(), req);
        assert_eq!(req.op.target(), "billing");
        assert_eq!(req.op.summary(), "add part billing");
    }
}
