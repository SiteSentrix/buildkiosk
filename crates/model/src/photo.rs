//! Photos: pictures of the work, kept in albums.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::project::Project;
use crate::record;

/// The picture types a photo may be. Only these raster formats are ever shown inline.
pub const IMAGE_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub id: Id<Photo>,
    pub project: Id<Project>,
    pub file: FileRef,
    #[serde(default)]
    pub caption: String,
    /// A named group: "Foundations", "Weekly progress".
    #[serde(default)]
    pub album: String,
    /// Where on site it was taken.
    #[serde(default)]
    pub location: String,
    /// The day it was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taken: Option<Date>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Photo, "photo", trash, project);
