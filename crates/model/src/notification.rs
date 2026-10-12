//! In-app notifications: a message for one person, with a link to what it is about.

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Person;
use crate::project::Project;
use crate::record;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: Id<Notification>,
    pub person: Id<Person>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Id<Project>>,
    pub text: String,
    /// A path inside the app, e.g. `/projects/1/submittals/2`.
    pub link: String,
    pub created: Stamp,
    #[serde(default)]
    pub read: bool,
}
record!(Notification, "notification");
