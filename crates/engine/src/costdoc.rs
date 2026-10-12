//! What change events, commitment change orders and budget changes share: a numbered record with a
//! title, a description and cost lines, that its author edits until it is settled and an admin can
//! always edit. The shared commands live here; each record adds its own transitions
//! (approve, close, void…) in its command module.

use buildkiosk_core::{Id, text};
use buildkiosk_model::finance::CostLine;
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::{CompanyState, InProject, Level, Record, Stamp, Table, Tool, Trash};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cmd::budget::usable_code;
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Actor, Ctx, EngineError, Params, Result, View};

/// A record that shares the cost document commands.
pub trait CostDoc: Record + InProject + Trash + Clone + PartialEq + Serialize + 'static {
    const TOOL: Tool;
    /// How messages name one: "change event".
    const NOUN: &'static str;
    /// Capitalised, for labels: "Change event".
    const LABEL: &'static str;

    fn table(state: &CompanyState) -> &Table<Self>;
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self>;
    /// A new record, numbered within the project, in its starting status.
    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self;
    fn set_id(&mut self, id: Id<Self>);
    fn number(&self) -> u32;
    fn title_mut(&mut self) -> &mut String;
    fn description_mut(&mut self) -> &mut String;
    fn lines(&self) -> &[CostLine];
    fn lines_mut(&mut self) -> &mut Vec<CostLine>;
    fn created(&self) -> Stamp;
    fn updated_mut(&mut self) -> &mut Stamp;
    /// Still being worked on, so its author may edit it (draft, or open).
    fn is_editable(&self) -> bool;
    fn is_void(&self) -> bool;
    /// Whether the record is private to its author and admins (a draft).
    fn is_private(&self) -> bool {
        false
    }
    /// Reads the fields only this record has.
    fn apply_extra(&mut self, _ctx: &Ctx<'_>, _p: &Params<'_>, _creating: bool) -> Result<()> {
        Ok(())
    }
}

/// "Change event #3".
pub fn label<T: CostDoc>(d: &T) -> String {
    format!("{} #{}", T::LABEL, d.number())
}

pub fn can_see<T: CostDoc>(state: &CompanyState, actor: Actor, d: &T) -> bool {
    let lvl = view::level(state, actor, d.project(), T::TOOL);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || d.deleted().is_some() {
        return false;
    }
    !d.is_private() || (actor.person().is_some() && d.created().by == actor.person())
}

/// The records of a project the actor may see, in number order.
pub fn docs_for<T: CostDoc>(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&T> {
    let mut list: Vec<&T> = T::table(state).iter().filter(|d| d.project() == project && d.deleted().is_none() && can_see(state, actor, *d)).collect();
    list.sort_by_key(|d| d.number());
    list
}

/// Whether the actor may edit the record now.
pub fn can_edit<T: CostDoc>(state: &CompanyState, actor: Actor, d: &T) -> bool {
    if d.is_void() {
        return false;
    }
    let lvl = view::level(state, actor, d.project(), T::TOOL);
    lvl == Level::Admin || (lvl >= Level::Standard && d.is_editable() && actor.person().is_some() && d.created().by == actor.person())
}

pub fn load<T: CostDoc>(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<T> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let d = in_project(T::table(ctx.state), project, id, T::NOUN)?;
    if !can_see(ctx.state, ctx.actor, d) {
        return Err(EngineError::not_found(format!("{} {id} not found in this project", T::NOUN)));
    }
    Ok(d.clone())
}

pub fn is_admin<T: CostDoc>(ctx: &Ctx<'_>, d: &T) -> bool {
    ctx.level(d.project(), T::TOOL) == Level::Admin
}

/// Reads a list of cost lines from `lines` (blank form rows are skipped).
pub fn read_lines(ctx: &Ctx<'_>, p: &Params<'_>, project: Id<Project>) -> Result<Vec<CostLine>> {
    const MAX_LINES: usize = 200;
    let rows = p.items("lines")?;
    if rows.len() > MAX_LINES {
        return Err(p.bad("lines", format!("holds more than {MAX_LINES} lines")));
    }
    let mut out = Vec::new();
    for row in rows {
        let code: Option<Id<CostCode>> = row.opt_id("cost_code")?;
        if code.is_none() && row.line_or_empty("amount", 40)?.is_empty() && row.line_or_empty("description", text::SHORT)?.is_empty() {
            continue;
        }
        let code = code.ok_or_else(|| row.bad("cost_code", "is required"))?;
        usable_code(ctx.state, project, code)?;
        let amount = row.money("amount")?.ok_or_else(|| row.bad("amount", "is required"))?;
        out.push(CostLine { cost_code: code, description: row.line_or_empty("description", text::SHORT)?, amount });
    }
    Ok(out)
}

fn apply_fields<T: CostDoc>(ctx: &Ctx<'_>, p: &Params<'_>, d: &mut T, creating: bool) -> Result<()> {
    if creating || p.present("title") {
        *d.title_mut() = p.req_line("title", text::SHORT)?;
    }
    if p.present("description") {
        *d.description_mut() = p.text_or_empty("description", text::LONG)?;
    }
    if p.present("lines") {
        let lines = read_lines(ctx, p, d.project())?;
        if buildkiosk_core::Money::sum(lines.iter().map(|l| l.amount)).is_none() {
            return Err(p.bad("lines", "the amounts are too large to add up"));
        }
        *d.lines_mut() = lines;
    }
    d.apply_extra(ctx, p, creating)
}

pub fn create<T: CostDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut d = T::blank(ctx, project);
    let number = d.number();
    apply_fields(ctx, p, &mut d, true)?;
    let id = T::table_mut(ctx.state).alloc();
    d.set_id(id);
    T::table_mut(ctx.state).put(d);
    Ok(json!({ "id": id, "number": number }))
}

pub fn update<T: CostDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d: T = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &d) {
        return Err(EngineError::forbidden(if d.is_void() {
            format!("{} is void", label(&d))
        } else {
            format!("only the author (until it is settled) or an admin can edit {}", label(&d))
        }));
    }
    let before = d.clone();
    apply_fields(ctx, p, &mut d, false)?;
    let id = d.id();
    if d != before {
        *d.updated_mut() = ctx.stamp();
        T::table_mut(ctx.state).put(d);
    }
    Ok(json!({ "id": id }))
}

pub fn delete<T: CostDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let d: T = load(ctx, p)?;
    let own = d.is_editable() && d.created().by.is_some() && d.created().by == ctx.person();
    if !is_admin(ctx, &d) && !own {
        return Err(EngineError::forbidden(format!("only admins can delete a {} (authors can delete their own until it is settled)", T::NOUN)));
    }
    let stamp = ctx.stamp();
    T::table_mut(ctx.state).update(d.id(), |x| x.set_deleted(Some(stamp)));
    Ok(json!({ "id": d.id() }))
}

pub fn restore<T: CostDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let d = in_project(T::table(ctx.state), project, id, T::NOUN)?;
    if d.deleted().is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", label(d))));
    }
    T::table_mut(ctx.state).update(id, |x| x.set_deleted(None));
    Ok(json!({ "id": id }))
}

pub fn list<T: CostDoc>(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, T::TOOL) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&T> = T::table(v.state).iter().filter(|d| d.project() == project && d.deleted().is_some()).collect();
        return json(&bin);
    }
    json(&docs_for::<T>(v.state, v.actor, project))
}

pub fn get<T: CostDoc>(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let d = in_project(T::table(v.state), project, id, T::NOUN)?;
    if !can_see(v.state, v.actor, d) {
        return Err(EngineError::not_found(format!("{} {id} not found in this project", T::NOUN)));
    }
    json(d)
}

/// Implements the field accessors of [`CostDoc`] for a record that has `number`, `title`,
/// `description`, `lines`, `created`, `updated` and `id`.
#[macro_export]
macro_rules! cost_doc_fields {
    () => {
        fn set_id(&mut self, id: buildkiosk_core::Id<Self>) {
            self.id = id;
        }
        fn number(&self) -> u32 {
            self.number
        }
        fn title_mut(&mut self) -> &mut String {
            &mut self.title
        }
        fn description_mut(&mut self) -> &mut String {
            &mut self.description
        }
        fn lines(&self) -> &[buildkiosk_model::finance::CostLine] {
            &self.lines
        }
        fn lines_mut(&mut self) -> &mut Vec<buildkiosk_model::finance::CostLine> {
            &mut self.lines
        }
        fn created(&self) -> buildkiosk_model::Stamp {
            self.created
        }
        fn updated_mut(&mut self) -> &mut buildkiosk_model::Stamp {
            &mut self.updated
        }
    };
}
