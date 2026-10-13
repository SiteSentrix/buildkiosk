//! Incidents: injuries, near misses and property damage reported on site.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::work::WorkCore;
use crate::{choice, record, work_deref};

pub use crate::work::WorkComment as IncidentComment;
pub use crate::work::WorkStatus as IncidentStatus;

choice! {
    /// How serious the outcome was.
    #[derive(Default)]
    pub enum Severity {
        #[default]
        NearMiss = ("near_miss", "Near miss"),
        PropertyDamage = ("property_damage", "Property damage"),
        FirstAid = ("first_aid", "First aid"),
        MedicalTreatment = ("medical_treatment", "Medical treatment"),
        LostTime = ("lost_time", "Lost time"),
        Fatality = ("fatality", "Fatality"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Incident {
    pub id: Id<Incident>,
    pub project: Id<Project>,
    #[serde(flatten)]
    pub core: WorkCore,
    /// The day it happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred: Option<Date>,
    #[serde(default)]
    pub severity: Severity,
    /// Whether it must be entered on the employer's injury log.
    #[serde(default)]
    pub recordable: bool,
    /// Who was hurt or involved, and who saw it.
    #[serde(default)]
    pub people_involved: String,
    #[serde(default)]
    pub root_cause: String,
    #[serde(default)]
    pub corrective_action: String,
}
record!(Incident, "incident", trash, project);
work_deref!(Incident);

impl Incident {
    pub fn label(&self) -> String {
        format!("Incident #{}", self.number)
    }
}
