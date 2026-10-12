//! Punch list: small items of work to fix or finish before a project closes out.

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::work::WorkCore;
use crate::{record, work_deref};

pub use crate::work::WorkComment as PunchComment;
pub use crate::work::WorkStatus as PunchStatus;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PunchItem {
    pub id: Id<PunchItem>,
    pub project: Id<Project>,
    #[serde(flatten)]
    pub core: WorkCore,
}
record!(PunchItem, "punch_item", trash, project);
work_deref!(PunchItem);

impl PunchItem {
    pub fn label(&self) -> String {
        format!("Punch item #{}", self.number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A row as the journal stored it before the shared `WorkCore` existed: every field flat.
    #[test]
    fn rows_written_before_the_shared_core_still_load_and_round_trip() {
        let old = json!({
            "id": 7, "project": 2, "number": 3, "title": "Touch up paint", "description": "", "location": "Corridor",
            "kind": "Paint", "priority": "high", "status": "ready_for_review", "manager": 1, "assignees": [4],
            "ball_in_court": [1], "due": "2026-10-30",
            "work_done": {"by": 4, "at": "2026-10-12T16:19:00Z"}, "attachments": [],
            "comments": [{"id": 1, "by": 4, "at": "2026-10-12T16:19:00Z", "body": "Done"}],
            "created": {"by": 1, "at": "2026-10-12T16:00:00Z"}, "updated": {"by": 4, "at": "2026-10-12T16:19:00Z"}
        });
        let item: PunchItem = serde_json::from_value(old.clone()).unwrap();
        assert_eq!((item.number, item.title.as_str(), item.status), (3, "Touch up paint", PunchStatus::ReadyForReview));
        assert_eq!(item.comments.len(), 1);
        assert_eq!(serde_json::to_value(&item).unwrap(), old, "the stored shape is unchanged");
    }
}
