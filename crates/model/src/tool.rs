//! The tool catalog and permission levels.
//!
//! A tool is one product area (RFIs, Budget, Daily Log…). Permission templates give each person a
//! [`Level`] per tool, and every command names the tool and level it needs. The catalog lists the
//! tools BuildKiosk has or plans; whether a tool is usable yet depends on the commands registered
//! for it (see the engine).

use crate::choice;

choice! {
    /// How much a person may do in a tool. Ordered: each level includes the ones below it.
    #[derive(Default)]
    pub enum Level {
        /// The tool is hidden.
        #[default]
        None = ("none", "None"),
        /// See items (except private ones they aren't part of).
        Read = ("read", "Read only"),
        /// Create items, edit their own, and act on items assigned to them.
        Standard = ("standard", "Standard"),
        /// Everything, including other people's items, settings, deleting and restoring.
        Admin = ("admin", "Admin"),
    }
}

choice! {
    /// Where a tool lives: on the company (all projects) or inside one project.
    pub enum Scope {
        Company = ("company", "Company"),
        Project = ("project", "Project"),
    }
}

choice! {
    /// How tools are grouped in navigation and permission templates.
    pub enum ToolGroup {
        Core = ("core", "Core tools"),
        ProjectManagement = ("project_management", "Project management"),
        QualitySafety = ("quality_safety", "Quality and safety"),
        Financials = ("financials", "Financial management"),
        Preconstruction = ("preconstruction", "Preconstruction"),
        Resources = ("resources", "Resource management"),
        Admin = ("admin", "Administration"),
    }
}

choice! {
    /// Every product area.
    pub enum Tool {
        // Company tools
        Portfolio = ("portfolio", "Portfolio"),
        Directory = ("directory", "Directory"),
        CompanyAdmin = ("company_admin", "Company admin"),
        CompanyReports = ("company_reports", "Company reports"),
        Equipment = ("equipment", "Equipment"),
        Prequalification = ("prequalification", "Prequalification"),
        // Project core tools
        ProjectHome = ("project_home", "Home"),
        ProjectDirectory = ("project_directory", "Project directory"),
        Documents = ("documents", "Documents"),
        Drawings = ("drawings", "Drawings"),
        Specifications = ("specifications", "Specifications"),
        Photos = ("photos", "Photos"),
        Schedule = ("schedule", "Schedule"),
        Tasks = ("tasks", "Tasks"),
        Reports = ("reports", "Reports"),
        // Project management
        Rfis = ("rfis", "RFIs"),
        Submittals = ("submittals", "Submittals"),
        Transmittals = ("transmittals", "Transmittals"),
        Meetings = ("meetings", "Meetings"),
        DailyLog = ("daily_log", "Daily log"),
        Correspondence = ("correspondence", "Correspondence"),
        Forms = ("forms", "Forms"),
        // Quality and safety
        PunchList = ("punch_list", "Punch list"),
        Inspections = ("inspections", "Inspections"),
        Observations = ("observations", "Observations"),
        Incidents = ("incidents", "Incidents"),
        ActionPlans = ("action_plans", "Action plans"),
        // Financial management
        Budget = ("budget", "Budget"),
        ChangeEvents = ("change_events", "Change events"),
        ChangeOrders = ("change_orders", "Change orders"),
        Commitments = ("commitments", "Commitments"),
        PrimeContracts = ("prime_contracts", "Prime contract"),
        Invoicing = ("invoicing", "Invoicing"),
        DirectCosts = ("direct_costs", "Direct costs"),
        // Preconstruction
        Bidding = ("bidding", "Bidding"),
        Estimating = ("estimating", "Estimating"),
        // Resource management
        Timesheets = ("timesheets", "Timesheets"),
        // Project administration
        ProjectAdmin = ("project_admin", "Project admin"),
    }
}

impl Tool {
    pub fn scope(self) -> Scope {
        match self {
            Tool::Portfolio | Tool::Directory | Tool::CompanyAdmin | Tool::CompanyReports | Tool::Equipment | Tool::Prequalification => {
                Scope::Company
            }
            _ => Scope::Project,
        }
    }

    pub fn group(self) -> ToolGroup {
        use Tool::*;
        match self {
            Portfolio | Directory | ProjectHome | ProjectDirectory | Documents | Drawings | Specifications | Photos | Schedule | Tasks | Reports
            | CompanyReports => ToolGroup::Core,
            Rfis | Submittals | Transmittals | Meetings | DailyLog | Correspondence | Forms => ToolGroup::ProjectManagement,
            PunchList | Inspections | Observations | Incidents | ActionPlans => ToolGroup::QualitySafety,
            Budget | ChangeEvents | ChangeOrders | Commitments | PrimeContracts | Invoicing | DirectCosts => ToolGroup::Financials,
            Bidding | Estimating | Prequalification => ToolGroup::Preconstruction,
            Timesheets | Equipment => ToolGroup::Resources,
            CompanyAdmin | ProjectAdmin => ToolGroup::Admin,
        }
    }

    /// The URL path segment for the tool (`rfis`, `daily-log`).
    pub fn slug(self) -> &'static str {
        use crate::Choice;
        match self {
            Tool::DailyLog => "daily-log",
            Tool::PunchList => "punch-list",
            Tool::ChangeEvents => "change-events",
            Tool::ChangeOrders => "change-orders",
            Tool::PrimeContracts => "prime-contract",
            Tool::DirectCosts => "direct-costs",
            Tool::ActionPlans => "action-plans",
            Tool::ProjectHome => "home",
            Tool::ProjectDirectory => "directory",
            Tool::ProjectAdmin => "admin",
            Tool::CompanyAdmin => "admin",
            Tool::CompanyReports => "reports",
            other => other.key(),
        }
    }

    /// Project tools, in navigation order.
    pub fn project_tools() -> impl Iterator<Item = Tool> {
        use crate::Choice;
        Tool::ALL.iter().copied().filter(|t| t.scope() == Scope::Project)
    }

    /// Company tools, in navigation order.
    pub fn company_tools() -> impl Iterator<Item = Tool> {
        use crate::Choice;
        Tool::ALL.iter().copied().filter(|t| t.scope() == Scope::Company)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Choice;

    #[test]
    fn levels_are_ordered() {
        assert!(Level::None < Level::Read && Level::Read < Level::Standard && Level::Standard < Level::Admin);
        assert_eq!(Level::parse("Read only"), Some(Level::Read));
        assert_eq!(Level::parse("ADMIN"), Some(Level::Admin));
    }

    #[test]
    fn keys_and_slugs_are_unique_within_a_scope() {
        for scope in Scope::ALL {
            let tools: Vec<Tool> = Tool::ALL.iter().copied().filter(|t| t.scope() == *scope).collect();
            let mut slugs: Vec<&str> = tools.iter().map(|t| t.slug()).collect();
            slugs.sort_unstable();
            let before = slugs.len();
            slugs.dedup();
            assert_eq!(before, slugs.len(), "duplicate slug in {scope:?}");
        }
        let mut keys: Vec<&str> = Tool::ALL.iter().map(|t| t.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Tool::ALL.len());
    }

    #[test]
    fn serde_uses_keys() {
        assert_eq!(serde_json::to_string(&Tool::DailyLog).unwrap(), "\"daily_log\"");
        assert_eq!(serde_json::from_str::<Tool>("\"rfis\"").unwrap(), Tool::Rfis);
        assert!(serde_json::from_str::<Tool>("\"nope\"").is_err());
    }
}
