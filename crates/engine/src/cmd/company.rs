//! The company: first-run setup and settings.

use std::collections::BTreeMap;

use buildkiosk_core::text;
use buildkiosk_model::directory::{Company, Person};
use buildkiosk_model::perm::PermissionTemplate;
use buildkiosk_model::project::CostCode;
use buildkiosk_model::{Level, Tool, ToolGroup};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::Read;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "company.setup",
        "Set up the company",
        Tool::CompanyAdmin,
        Access::System,
        "name: text; slug?: text (URL id, made from the name if absent); admin_first_name: text; admin_last_name: text; admin_email: email; \
         utc_offset_minutes?: integer (-720..840); currency?: text (ISO code, default USD); standard_cost_codes?: bool (default true)",
        setup,
    ),
    CommandSpec::mutate(
        "company.update",
        "Edit company settings",
        Tool::CompanyAdmin,
        Access::Company(Level::Admin),
        "name?, phone?, website?, street?, city?, region?, postal_code?, country?: text; utc_offset_minutes?: integer; currency?: text; \
         rfi_due_days?: integer (1..90); submittal_review_days?: integer (1..90)",
        update,
    ),
    CommandSpec::query("company.get", "Show company settings", Tool::CompanyAdmin, Access::Member, "(none)", get),
];

/// The default templates a new company starts with. Original to BuildKiosk; companies edit them.
fn default_templates() -> Vec<(&'static str, &'static str, BTreeMap<Tool, Level>)> {
    let project_tools: Vec<Tool> = Tool::project_tools().collect();
    let all = |level: Level| -> BTreeMap<Tool, Level> { project_tools.iter().map(|t| (*t, level)).collect() };
    let with = |base: Level, rules: &[(ToolGroup, Level)], extra: &[(Tool, Level)]| -> BTreeMap<Tool, Level> {
        let mut m: BTreeMap<Tool, Level> = BTreeMap::new();
        for t in &project_tools {
            let l = rules.iter().find(|(g, _)| *g == t.group()).map(|(_, l)| *l).unwrap_or(base);
            m.insert(*t, l);
        }
        for (t, l) in extra {
            m.insert(*t, *l);
        }
        m.retain(|_, l| *l != Level::None);
        m
    };
    let mut pm = all(Level::Admin);
    pm.insert(Tool::Directory, Level::Standard);
    pm.insert(Tool::Portfolio, Level::Standard);
    vec![
        ("Project manager", "Runs projects: admin on every project tool, can create projects and add people.", pm),
        (
            "Superintendent",
            "Runs the field: admin on quality, safety and daily tools, standard on project management, read-only financials.",
            with(
                Level::Standard,
                &[(ToolGroup::QualitySafety, Level::Admin), (ToolGroup::Financials, Level::Read), (ToolGroup::Preconstruction, Level::Read)],
                &[(Tool::DailyLog, Level::Admin), (Tool::Photos, Level::Admin), (Tool::Schedule, Level::Standard), (Tool::Directory, Level::Read)],
            ),
        ),
        (
            "Project engineer",
            "Handles RFIs, submittals and documents: standard on project management, read-only financials.",
            with(
                Level::Standard,
                &[(ToolGroup::Financials, Level::Read), (ToolGroup::Preconstruction, Level::Read), (ToolGroup::Admin, Level::Read)],
                &[(Tool::Directory, Level::Read)],
            ),
        ),
        (
            "Subcontractor",
            "Trade partners: can raise and answer RFIs, send submittals, work punch items and tasks; reads drawings and the schedule; no financials.",
            with(
                Level::Read,
                &[(ToolGroup::Financials, Level::None), (ToolGroup::Preconstruction, Level::None), (ToolGroup::Admin, Level::None)],
                &[
                    (Tool::Rfis, Level::Standard),
                    (Tool::Submittals, Level::Standard),
                    (Tool::PunchList, Level::Standard),
                    (Tool::Observations, Level::Standard),
                    (Tool::Tasks, Level::Standard),
                    (Tool::DailyLog, Level::None),
                    (Tool::Incidents, Level::None),
                ],
            ),
        ),
        (
            "Owner / design team",
            "Owners, architects and engineers: answer RFIs and review submittals; read the rest except internal cost tools.",
            with(
                Level::Read,
                &[(ToolGroup::Preconstruction, Level::None), (ToolGroup::Admin, Level::None)],
                &[
                    (Tool::Rfis, Level::Standard),
                    (Tool::Submittals, Level::Standard),
                    (Tool::Budget, Level::None),
                    (Tool::Commitments, Level::None),
                    (Tool::DirectCosts, Level::None),
                    (Tool::ChangeEvents, Level::None),
                    (Tool::Timesheets, Level::None),
                    (Tool::DailyLog, Level::None),
                ],
            ),
        ),
        (
            "Read only",
            "Sees every project tool except financials; changes nothing.",
            with(Level::Read, &[(ToolGroup::Financials, Level::None), (ToolGroup::Preconstruction, Level::None)], &[]),
        ),
    ]
}

/// A small starter cost breakdown, original to BuildKiosk. Companies replace it with their own list
/// (`company.cost_code.import`).
pub const STARTER_COST_CODES: &[(&str, &str)] = &[
    ("01-100", "General requirements"),
    ("01-500", "Temporary facilities and controls"),
    ("02-100", "Site preparation and demolition"),
    ("03-100", "Concrete forming"),
    ("03-300", "Cast-in-place concrete"),
    ("04-200", "Masonry"),
    ("05-100", "Structural steel"),
    ("06-100", "Rough carpentry"),
    ("06-200", "Finish carpentry and millwork"),
    ("07-200", "Insulation"),
    ("07-500", "Roofing"),
    ("08-100", "Doors and frames"),
    ("08-500", "Windows and glazing"),
    ("09-200", "Drywall and framing"),
    ("09-600", "Flooring"),
    ("09-900", "Painting and coatings"),
    ("10-100", "Specialties"),
    ("21-100", "Fire suppression"),
    ("22-100", "Plumbing"),
    ("23-100", "Heating, ventilation and air conditioning"),
    ("26-100", "Electrical"),
    ("27-100", "Communications and low voltage"),
    ("31-200", "Earthwork"),
    ("32-100", "Paving and site improvements"),
    ("33-100", "Site utilities"),
];

fn setup(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    if ctx.state.company().is_some() {
        return Err(EngineError::conflict("the company is already set up"));
    }
    let name = p.req_line("name", text::SHORT)?;
    let slug = match p.line("slug", 63)? {
        Some(s) => text::slug(&s, 63),
        None => text::slug(&name, 63),
    };
    if slug.is_empty() {
        return Err(p.bad("slug", "must contain letters or digits"));
    }
    let offset = p.int("utc_offset_minutes", -720..=840)?.unwrap_or(0);
    let currency = p.line("currency", 3)?.unwrap_or_else(|| "USD".into()).to_ascii_uppercase();
    let stamp = ctx.stamp();
    ctx.state.company.put(Company {
        id: Company::ID,
        name,
        slug: slug.clone(),
        address: Default::default(),
        phone: String::new(),
        website: String::new(),
        utc_offset_minutes: i32::try_from(offset).unwrap_or(0),
        currency,
        rfi_due_days: 7,
        submittal_review_days: 10,
        require_prequalification: false,
        created: stamp,
        updated: stamp,
    });
    for (name, description, levels) in default_templates() {
        let id = ctx.state.templates.alloc();
        ctx.state.templates.put(PermissionTemplate {
            id,
            name: name.into(),
            description: description.into(),
            levels,
            created: stamp,
            updated: stamp,
            deleted: None,
        });
    }
    let email = p.email("admin_email")?.ok_or_else(|| p.bad("admin_email", "is required"))?;
    let admin = ctx.state.people.alloc();
    ctx.state.people.put(Person {
        id: admin,
        first_name: p.req_line("admin_first_name", 100)?,
        last_name: p.req_line("admin_last_name", 100)?,
        email: Some(email),
        phone: String::new(),
        title: "Administrator".into(),
        vendor: None,
        is_user: true,
        company_admin: true,
        company_template: None,
        active: true,
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    if p.flag_or("standard_cost_codes", true)? {
        for (code, description) in STARTER_COST_CODES {
            let id = ctx.state.cost_codes.alloc();
            ctx.state.cost_codes.put(CostCode {
                id,
                project: None,
                code: (*code).into(),
                description: (*description).into(),
                created: stamp,
                deleted: None,
            });
        }
    }
    Ok(json!({ "slug": slug, "admin": admin }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let before = ctx.state.company().cloned().ok_or_else(|| EngineError::conflict("the company isn't set up yet"))?;
    let mut c = before.clone();
    if p.present("name") {
        c.name = p.req_line("name", text::SHORT)?;
    }
    for (key, field) in [
        ("phone", &mut c.phone),
        ("website", &mut c.website),
        ("street", &mut c.address.street),
        ("city", &mut c.address.city),
        ("region", &mut c.address.region),
        ("postal_code", &mut c.address.postal_code),
        ("country", &mut c.address.country),
    ] {
        if p.present(key) {
            *field = p.line_or_empty(key, text::SHORT)?;
        }
    }
    if let Some(o) = p.int("utc_offset_minutes", -720..=840)? {
        c.utc_offset_minutes = i32::try_from(o).unwrap_or(0);
    }
    if let Some(cur) = p.line("currency", 3)? {
        if cur.len() != 3 || !cur.chars().all(|ch| ch.is_ascii_alphabetic()) {
            return Err(p.bad("currency", "must be a three-letter ISO code such as USD"));
        }
        c.currency = cur.to_ascii_uppercase();
    }
    if let Some(d) = p.int("rfi_due_days", 1..=90)? {
        c.rfi_due_days = u32::try_from(d).unwrap_or(7);
    }
    if let Some(d) = p.int("submittal_review_days", 1..=90)? {
        c.submittal_review_days = u32::try_from(d).unwrap_or(10);
    }
    if c != before {
        c.updated = ctx.stamp();
        ctx.state.company.put(c);
    }
    Ok(json!({}))
}

fn get(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    let c = v.state().company().ok_or_else(|| EngineError::not_found("the company isn't set up yet"))?;
    json(c)
}

/// The template ids by name, for seeding and tests.
pub fn template_named(state: &buildkiosk_model::CompanyState, name: &str) -> Option<buildkiosk_core::Id<PermissionTemplate>> {
    state.templates.iter().find(|t| t.deleted.is_none() && t.name.eq_ignore_ascii_case(name)).map(|t| t.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_templates_are_sane() {
        let t = default_templates();
        assert_eq!(t.len(), 6);
        let sub = &t.iter().find(|(n, _, _)| *n == "Subcontractor").unwrap().2;
        assert_eq!(sub.get(&Tool::Rfis), Some(&Level::Standard));
        assert_eq!(sub.get(&Tool::Budget), None);
        let pm = &t.iter().find(|(n, _, _)| *n == "Project manager").unwrap().2;
        assert!(Tool::project_tools().all(|tool| pm.get(&tool) == Some(&Level::Admin)));
        use buildkiosk_model::Choice;
        for (_, _, levels) in &t {
            assert!(levels.values().all(|l| *l != Level::None));
            assert!(levels.keys().all(|k| Tool::parse(k.key()).is_some()));
        }
    }
}
