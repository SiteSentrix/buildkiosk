//! Incidents: injuries, near misses and property damage reported on site.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::directory::Person;
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

impl Severity {
    /// Serious enough that the people who run the tool are told when it is reported: medical treatment or worse.
    pub fn is_serious(self) -> bool {
        matches!(self, Severity::MedicalTreatment | Severity::LostTime | Severity::Fatality)
    }

    /// Whether the case counts toward lost-time rates.
    pub fn is_lost_time(self) -> bool {
        matches!(self, Severity::LostTime | Severity::Fatality)
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
    /// A private incident is seen only by its author, its manager, the people assigned, the people named in
    /// `involved` and the tool's admins.
    #[serde(default)]
    pub private: bool,
    /// Who may see a private incident besides the author, manager and assignees (the people it happened to).
    #[serde(default)]
    pub involved: Vec<Id<Person>>,
}
record!(Incident, "incident", trash, project);
work_deref!(Incident);

impl Incident {
    pub fn label(&self) -> String {
        format!("Incident #{}", self.number)
    }
}
