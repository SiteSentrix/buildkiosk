//! The company directory: the company itself, people, other companies and distribution groups.

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::common::{Address, Stamp};
use crate::perm::PermissionTemplate;
use crate::project::Project;
use crate::{choice, record};

/// The company this data belongs to (one row, id 1): the tenant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Company {
    pub id: Id<Company>,
    pub name: String,
    /// URL-safe identifier ("summit-ridge-builders").
    pub slug: String,
    #[serde(default)]
    pub address: Address,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub website: String,
    /// Minutes east of UTC, for "today" and displayed times (a fixed offset for now: see gaps.md).
    #[serde(default)]
    pub utc_offset_minutes: i32,
    /// ISO 4217 code of the currency all amounts are in.
    #[serde(default = "default_currency")]
    pub currency: String,
    /// Working days from opening an RFI to its default due date.
    #[serde(default = "default_rfi_days")]
    pub rfi_due_days: u32,
    /// Working days a submittal reviewer gets by default.
    #[serde(default = "default_submittal_days")]
    pub submittal_review_days: u32,
    /// A commitment can only be approved, and a bid awarded, to a company whose prequalification is current.
    #[serde(default)]
    pub require_prequalification: bool,
    pub created: Stamp,
    pub updated: Stamp,
}
record!(Company, "company");

fn default_currency() -> String {
    "USD".to_string()
}

fn default_rfi_days() -> u32 {
    7
}

fn default_submittal_days() -> u32 {
    10
}

impl Company {
    pub const ID: Id<Company> = Id::new(1);
}

/// A person in the directory: an employee, or someone at another company. `is_user` people can
/// sign in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: Id<Person>,
    pub first_name: String,
    pub last_name: String,
    /// Lowercase; unique among people who can sign in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub title: String,
    /// The company they work for; `None` means our own company.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<Id<Vendor>>,
    /// May sign in (credentials live outside the journal, in the server's credential store).
    #[serde(default)]
    pub is_user: bool,
    /// Admin on every tool of every project.
    #[serde(default)]
    pub company_admin: bool,
    /// Permissions on company-level tools (Directory, Company admin…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_template: Option<Id<PermissionTemplate>>,
    #[serde(default = "yes")]
    pub active: bool,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Person, "person", trash);

fn yes() -> bool {
    true
}

impl Person {
    pub fn name(&self) -> String {
        match (self.first_name.is_empty(), self.last_name.is_empty()) {
            (false, false) => format!("{} {}", self.first_name, self.last_name),
            (false, true) => self.first_name.clone(),
            (true, false) => self.last_name.clone(),
            (true, true) => self.email.clone().unwrap_or_else(|| format!("Person {}", self.id)),
        }
    }

    /// May act at all: active, not deleted.
    pub fn is_enabled(&self) -> bool {
        self.active && self.deleted.is_none()
    }
}

choice! {
    /// What another company is to us.
    #[derive(Default)]
    pub enum VendorKind {
        #[default]
        Subcontractor = ("subcontractor", "Subcontractor"),
        Supplier = ("supplier", "Supplier"),
        Owner = ("owner", "Owner / client"),
        Design = ("design", "Architect / engineer"),
        Consultant = ("consultant", "Consultant"),
        Inspector = ("inspector", "Inspector / agency"),
        Other = ("other", "Other"),
    }
}

/// Another company: subcontractor, supplier, owner, designer…
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vendor {
    pub id: Id<Vendor>,
    pub name: String,
    #[serde(default)]
    pub kind: VendorKind,
    /// Trade or specialty ("Electrical", "Concrete").
    #[serde(default)]
    pub trade: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub website: String,
    #[serde(default)]
    pub address: Address,
    #[serde(default)]
    pub license_number: String,
    #[serde(default)]
    pub notes: String,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Vendor, "vendor", trash);

/// A named list of people to copy on items (company-wide, or for one project).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DistributionGroup {
    pub id: Id<DistributionGroup>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Id<Project>>,
    #[serde(default)]
    pub members: Vec<Id<Person>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(DistributionGroup, "distribution_group", trash);
