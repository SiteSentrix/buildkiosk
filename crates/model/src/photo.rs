//! Photos: pictures of the work, kept in albums.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::observation::Observation;
use crate::project::Project;
use crate::punch::PunchItem;
use crate::record;
use crate::rfi::Rfi;

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
    /// Pixels, read from the picture when it was added (0 when unknown).
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// The camera's make and model from the picture's metadata, if it has any.
    #[serde(default)]
    pub camera: String,
    /// Things the picture is evidence for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rfis: Vec<Id<Rfi>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<Id<Observation>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub punch_items: Vec<Id<PunchItem>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Photo, "photo", trash, project);
