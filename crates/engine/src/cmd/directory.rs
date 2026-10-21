//! The company directory: people, companies (vendors) and distribution groups.

use buildkiosk_core::{Id, text};
use buildkiosk_model::directory::{DistributionGroup, Person, Vendor, VendorKind};
use buildkiosk_model::perm::PermissionTemplate;
use buildkiosk_model::{CompanyState, Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::Read;
use crate::view::json;
use crate::{Actor, Ctx, EngineError, Params, Result, View};

const PERSON_FIELDS: &str = "first_name, last_name: text; email?: email; phone?, title?: text; vendor?: company id (empty = our company); \
     is_user?: bool (may sign in; Directory admin); company_admin?: bool (company admins only); company_template?: permission template id";

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "person.set_language",
        "Choose my language",
        Tool::Directory,
        Access::Member,
        "language: en|es|fr|nl, or empty to follow the browser (changes the calling person's own setting)",
        set_language,
    ),
    CommandSpec::mutate("person.create", "Add person", Tool::Directory, Access::Company(Level::Standard), PERSON_FIELDS, person_create),
    CommandSpec::mutate(
        "person.update",
        "Edit person",
        Tool::Directory,
        Access::Company(Level::Standard),
        "id: person id; then any of the person.create fields, plus active?: bool",
        person_update,
    ),
    CommandSpec::mutate("person.delete", "Delete person", Tool::Directory, Access::Company(Level::Admin), "id: person id", person_delete),
    CommandSpec::mutate("person.restore", "Restore person", Tool::Directory, Access::Company(Level::Admin), "id: person id", person_restore),
    CommandSpec::query(
        "person.list",
        "List people",
        Tool::Directory,
        Access::Company(Level::Read),
        "q?: text (name, email, title); vendor?: company id; users_only?: bool; include_inactive?: bool",
        person_list,
    ),
    CommandSpec::query("person.get", "Show person", Tool::Directory, Access::Company(Level::Read), "id: person id", person_get),
    CommandSpec::mutate(
        "vendor.create",
        "Add company",
        Tool::Directory,
        Access::Company(Level::Standard),
        "name: text; kind?: subcontractor|supplier|owner|design|consultant|inspector|other; trade?, email?, phone?, website?, street?, city?, \
         region?, postal_code?, country?, license_number?: text; notes?: text",
        vendor_create,
    ),
    CommandSpec::mutate(
        "vendor.update",
        "Edit company",
        Tool::Directory,
        Access::Company(Level::Standard),
        "id: company id; then any of the vendor.create fields",
        vendor_update,
    ),
    CommandSpec::mutate("vendor.delete", "Delete company", Tool::Directory, Access::Company(Level::Admin), "id: company id", vendor_delete),
    CommandSpec::mutate(
        "vendor.merge",
        "Merge duplicate companies",
        Tool::Directory,
        Access::Company(Level::Admin),
        "from: company id (the duplicate, removed afterwards); into: company id (kept). Everything that pointed at `from` points at `into`.",
        vendor_merge,
    ),
    CommandSpec::mutate("vendor.restore", "Restore company", Tool::Directory, Access::Company(Level::Admin), "id: company id", vendor_restore),
    CommandSpec::query("vendor.list", "List companies", Tool::Directory, Access::Company(Level::Read), "q?: text; kind?: vendor kind", vendor_list),
    CommandSpec::query("vendor.get", "Show company", Tool::Directory, Access::Company(Level::Read), "id: company id", vendor_get),
    CommandSpec::mutate(
        "group.create",
        "Add distribution group",
        Tool::Directory,
        Access::Company(Level::Standard),
        "name: text; members?: [person ids]",
        group_create,
    ),
    CommandSpec::mutate(
        "group.update",
        "Edit distribution group",
        Tool::Directory,
        Access::Company(Level::Standard),
        "id: group id; name?: text; members?: [person ids]",
        group_update,
    ),
    CommandSpec::mutate("group.delete", "Delete distribution group", Tool::Directory, Access::Company(Level::Standard), "id: group id", group_delete),
    CommandSpec::query("group.list", "List distribution groups", Tool::Directory, Access::Member, "(none)", group_list),
];

fn email_taken(state: &CompanyState, email: &str, except: Option<Id<Person>>) -> bool {
    state.people.iter().any(|p| p.deleted.is_none() && Some(p.id) != except && p.email.as_deref() == Some(email))
}

/// Enabled company admins other than `except`.
fn other_admins(state: &CompanyState, except: Id<Person>) -> usize {
    state.people.iter().filter(|p| p.id != except && p.company_admin && p.is_user && p.is_enabled()).count()
}

fn is_company_admin(ctx: &Ctx<'_>) -> bool {
    match ctx.actor {
        Actor::System => true,
        Actor::Person(p) => ctx.state.people.get(p).is_some_and(|x| x.company_admin),
    }
}

fn check_template(ctx: &Ctx<'_>, id: Option<Id<PermissionTemplate>>) -> Result<()> {
    match id {
        Some(t) if ctx.state.templates.get(t).is_none_or(|x| x.deleted.is_some()) => Err(ctx.bad(format!("permission template {t} not found"))),
        _ => Ok(()),
    }
}

/// Applies the person fields present in `p` to `person`.
fn apply_person(ctx: &Ctx<'_>, p: &Params<'_>, person: &mut Person, creating: bool) -> Result<()> {
    if creating || p.present("first_name") {
        person.first_name = p.line_or_empty("first_name", 100)?;
    }
    if creating || p.present("last_name") {
        person.last_name = p.line_or_empty("last_name", 100)?;
    }
    if person.first_name.is_empty() && person.last_name.is_empty() {
        return Err(ctx.bad("a person needs a first or last name"));
    }
    if p.present("email") {
        let email = p.email("email")?;
        if let Some(e) = &email
            && email_taken(ctx.state, e, Some(person.id))
        {
            return Err(p.bad("email", format!("{e} is already in the directory")));
        }
        person.email = email;
    }
    if p.present("phone") {
        person.phone = p.line_or_empty("phone", 50)?;
    }
    if p.present("title") {
        person.title = p.line_or_empty("title", 100)?;
    }
    if p.present("vendor") {
        let v = p.opt_id::<Vendor>("vendor")?;
        if let Some(v) = v {
            ctx.check_vendor(v)?;
        }
        person.vendor = v;
    }
    let directory_admin = ctx.company_level(Tool::Directory) == Level::Admin;
    if let Some(u) = p.flag("is_user")?
        && u != person.is_user
    {
        if !directory_admin {
            return Err(EngineError::forbidden("only Directory admins can give or remove sign-in access"));
        }
        if u && person.email.is_none() {
            return Err(p.bad("is_user", "people who sign in need an email address"));
        }
        person.is_user = u;
    }
    if p.present("company_template") {
        if !directory_admin {
            return Err(EngineError::forbidden("only Directory admins can change company permissions"));
        }
        let t = p.opt_id("company_template")?;
        check_template(ctx, t)?;
        person.company_template = t;
    }
    if let Some(a) = p.flag("company_admin")?
        && a != person.company_admin
    {
        if !is_company_admin(ctx) {
            return Err(EngineError::forbidden("only company admins can make or remove company admins"));
        }
        if a && !person.is_user {
            return Err(p.bad("company_admin", "a company admin must be able to sign in (set is_user)"));
        }
        person.company_admin = a;
    }
    if let Some(active) = p.flag("active")? {
        if !directory_admin {
            return Err(EngineError::forbidden("only Directory admins can deactivate people"));
        }
        person.active = active;
    }
    if person.is_user && person.email.is_none() {
        return Err(p.bad("email", "people who sign in need an email address"));
    }
    Ok(())
}

/// Refuses changes that would leave the company without an enabled admin who can sign in.
fn keep_an_admin(ctx: &Ctx<'_>, before: &Person, after: &Person) -> Result<()> {
    let was = before.company_admin && before.is_user && before.is_enabled();
    let is = after.company_admin && after.is_user && after.is_enabled();
    if was && !is && other_admins(ctx.state, before.id) == 0 {
        return Err(EngineError::conflict("the company needs at least one active company admin who can sign in"));
    }
    Ok(())
}

fn set_language(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let me = ctx.person().ok_or_else(|| EngineError::forbidden("only a signed-in person has a language"))?;
    let code = p.line_or_empty("language", 10)?.to_lowercase();
    if !code.is_empty() && !buildkiosk_model::directory::LANGUAGES.contains(&code.as_str()) {
        return Err(p.bad("language", "must be en, es, fr or nl (empty follows the browser)"));
    }
    let _ = ctx.state.people.update(me, |x| x.language = code.clone());
    Ok(json!({ "language": code }))
}

fn person_create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let stamp = ctx.stamp();
    let mut person = Person {
        id: ctx.state.people.peek_next(),
        first_name: String::new(),
        last_name: String::new(),
        email: None,
        phone: String::new(),
        title: String::new(),
        vendor: None,
        is_user: false,
        company_admin: false,
        company_template: None,
        active: true,
        language: String::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_person(ctx, p, &mut person, true)?;
    let id = ctx.state.people.alloc();
    person.id = id;
    ctx.state.people.put(person);
    Ok(json!({ "id": id }))
}

fn person_update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Person> = p.id("id")?;
    let before =
        ctx.state.people.get(id).filter(|x| x.deleted.is_none()).cloned().ok_or_else(|| EngineError::not_found(format!("person {id} not found")))?;
    let mut person = before.clone();
    apply_person(ctx, p, &mut person, false)?;
    keep_an_admin(ctx, &before, &person)?;
    if person != before {
        person.updated = ctx.stamp();
        ctx.state.people.put(person);
    }
    Ok(json!({ "id": id }))
}

fn person_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Person> = p.id("id")?;
    let before =
        ctx.state.people.get(id).filter(|x| x.deleted.is_none()).cloned().ok_or_else(|| EngineError::not_found(format!("person {id} not found")))?;
    if ctx.actor.person() == Some(id) {
        return Err(EngineError::conflict("you can't delete yourself"));
    }
    let mut person = before.clone();
    person.deleted = Some(ctx.stamp());
    keep_an_admin(ctx, &before, &person)?;
    ctx.state.people.put(person);
    Ok(json!({ "id": id }))
}

fn person_restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Person> = p.id("id")?;
    let person = ctx
        .state
        .people
        .get(id)
        .filter(|x| x.deleted.is_some())
        .ok_or_else(|| EngineError::not_found(format!("person {id} is not in the recycle bin")))?;
    if let Some(e) = person.email.clone()
        && email_taken(ctx.state, &e, Some(id))
    {
        return Err(EngineError::conflict(format!("{e} now belongs to someone else in the directory")));
    }
    let stamp = ctx.stamp();
    ctx.state.people.update(id, |x| {
        x.deleted = None;
        x.updated = stamp;
    });
    Ok(json!({ "id": id }))
}

fn person_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let q = p.line("q", 100)?.map(|s| s.to_lowercase());
    let vendor = p.opt_id::<Vendor>("vendor")?;
    let users_only = p.flag_or("users_only", false)?;
    let inactive = p.flag_or("include_inactive", false)?;
    let list: Vec<&Person> = v
        .state
        .people
        .iter()
        .filter(|x| x.deleted.is_none() && (inactive || x.active))
        .filter(|x| !users_only || x.is_user)
        .filter(|x| vendor.is_none() || x.vendor == vendor)
        .filter(|x| {
            q.as_ref().is_none_or(|q| {
                x.name().to_lowercase().contains(q) || x.title.to_lowercase().contains(q) || x.email.as_deref().is_some_and(|e| e.contains(q))
            })
        })
        .collect();
    json(&list)
}

fn person_get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Person> = p.id("id")?;
    json(v.state.people.get(id).ok_or_else(|| EngineError::not_found(format!("person {id} not found")))?)
}

fn apply_vendor(p: &Params<'_>, x: &mut Vendor, creating: bool) -> Result<()> {
    if creating || p.present("name") {
        x.name = p.req_line("name", text::SHORT)?;
    }
    if let Some(k) = p.choice::<VendorKind>("kind")? {
        x.kind = k;
    }
    for (key, field, max) in [
        ("trade", &mut x.trade, 100),
        ("email", &mut x.email, text::EMAIL),
        ("phone", &mut x.phone, 50),
        ("website", &mut x.website, text::SHORT),
        ("street", &mut x.address.street, text::SHORT),
        ("city", &mut x.address.city, 100),
        ("region", &mut x.address.region, 100),
        ("postal_code", &mut x.address.postal_code, 20),
        ("country", &mut x.address.country, 100),
        ("license_number", &mut x.license_number, 100),
    ] {
        if p.present(key) {
            *field = p.line_or_empty(key, max)?;
        }
    }
    if !x.email.is_empty() {
        x.email = text::email(&x.email).ok_or_else(|| p.bad("email", "is not a valid email address"))?;
    }
    if p.present("notes") {
        x.notes = p.text_or_empty("notes", text::LONG)?;
    }
    Ok(())
}

fn vendor_create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let stamp = ctx.stamp();
    let mut x = Vendor {
        id: ctx.state.vendors.peek_next(),
        name: String::new(),
        kind: VendorKind::default(),
        trade: String::new(),
        email: String::new(),
        phone: String::new(),
        website: String::new(),
        address: Default::default(),
        license_number: String::new(),
        notes: String::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_vendor(p, &mut x, true)?;
    if ctx.state.vendors.iter().any(|v| v.deleted.is_none() && v.name.eq_ignore_ascii_case(&x.name)) {
        return Err(p.bad("name", format!("{} is already in the directory", x.name)));
    }
    let id = ctx.state.vendors.alloc();
    x.id = id;
    ctx.state.vendors.put(x);
    Ok(json!({ "id": id }))
}

fn vendor_update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Vendor> = p.id("id")?;
    let before = ctx
        .state
        .vendors
        .get(id)
        .filter(|v| v.deleted.is_none())
        .cloned()
        .ok_or_else(|| EngineError::not_found(format!("company {id} not found")))?;
    let mut x = before.clone();
    apply_vendor(p, &mut x, false)?;
    if ctx.state.vendors.iter().any(|v| v.id != id && v.deleted.is_none() && v.name.eq_ignore_ascii_case(&x.name)) {
        return Err(p.bad("name", format!("{} is already in the directory", x.name)));
    }
    if x != before {
        x.updated = ctx.stamp();
        ctx.state.vendors.put(x);
    }
    Ok(json!({ "id": id }))
}

fn vendor_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Vendor> = p.id("id")?;
    let stamp = ctx.stamp();
    ctx.state
        .vendors
        .update(id, |v| {
            if v.deleted.is_none() {
                v.deleted = Some(stamp)
            }
        })
        .ok_or_else(|| EngineError::not_found(format!("company {id} not found")))?;
    Ok(json!({ "id": id }))
}

/// Points everything that referred to the duplicate company at the one kept, then removes the
/// duplicate. Blank details of the kept company are filled in from the duplicate.
fn vendor_merge(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let from: Id<Vendor> = p.id("from")?;
    let into: Id<Vendor> = p.id("into")?;
    if from == into {
        return Err(p.bad("into", "must be a different company"));
    }
    ctx.check_vendor(from)?;
    ctx.check_vendor(into)?;
    let s = &mut *ctx.state;
    let swap = |v: &mut Id<Vendor>| {
        if *v == from {
            *v = into;
        }
    };
    let swap_opt = |v: &mut Option<Id<Vendor>>| {
        if *v == Some(from) {
            *v = Some(into);
        }
    };
    let mut moved = 0usize;
    macro_rules! repoint {
        ($table:ident, $field:ident) => {
            let ids: Vec<_> = s.$table.iter().filter(|r| r.$field == Some(from)).map(|r| r.id).collect();
            moved += ids.len();
            for id in ids {
                s.$table.update(id, |r| swap_opt(&mut r.$field));
            }
        };
    }
    repoint!(people, vendor);
    repoint!(rfis, responsible_vendor);
    repoint!(projects, owner);
    repoint!(schedule_tasks, company);
    repoint!(commitments, vendor);
    repoint!(direct_costs, vendor);
    repoint!(prime_contracts, owner);

    // Bid packages: invitations, bids (one per company: the later one stands) and the award.
    let ids: Vec<_> = s
        .bid_packages
        .iter()
        .filter(|b| b.invited.contains(&from) || b.awarded_to == Some(from) || b.bids.iter().any(|x| x.vendor == from))
        .map(|b| b.id)
        .collect();
    moved += ids.len();
    for id in ids {
        s.bid_packages.update(id, |b| {
            b.invited.iter_mut().for_each(swap);
            let mut seen = std::collections::BTreeSet::new();
            b.invited.retain(|v| seen.insert(v.raw()));
            swap_opt(&mut b.awarded_to);
            let kept_later = b.bids.iter().filter(|x| x.vendor == into).map(|x| x.received).max();
            let dup_later = b.bids.iter().filter(|x| x.vendor == from).map(|x| x.received).max();
            if let (Some(k), Some(d)) = (kept_later, dup_later) {
                if d > k {
                    b.bids.retain(|x| x.vendor != into);
                } else {
                    b.bids.retain(|x| x.vendor != from);
                }
            }
            b.bids.iter_mut().for_each(|x| swap(&mut x.vendor));
        });
    }

    // Prequalification: one live record per company. The kept company's stands; otherwise the
    // duplicate's moves over.
    let kept_has = s.prequalifications.iter().any(|q| q.vendor == into && q.deleted.is_none());
    let stamp = ctx.stamp();
    let s = &mut *ctx.state;
    let ids: Vec<_> = s.prequalifications.iter().filter(|q| q.vendor == from).map(|q| q.id).collect();
    for id in ids {
        s.prequalifications.update(id, |q| {
            if kept_has && q.deleted.is_none() {
                q.deleted = Some(stamp);
            }
            q.vendor = into;
        });
    }

    // Fill the kept company's blanks, then retire the duplicate.
    let Some(dup) = s.vendors.get(from).cloned() else { return Err(EngineError::not_found(format!("company {from} not found"))) };
    s.vendors.update(into, |k| {
        for (mine, theirs) in [
            (&mut k.trade, &dup.trade),
            (&mut k.email, &dup.email),
            (&mut k.phone, &dup.phone),
            (&mut k.website, &dup.website),
            (&mut k.license_number, &dup.license_number),
            (&mut k.address.street, &dup.address.street),
            (&mut k.address.city, &dup.address.city),
            (&mut k.address.region, &dup.address.region),
            (&mut k.address.postal_code, &dup.address.postal_code),
            (&mut k.address.country, &dup.address.country),
        ] {
            if mine.trim().is_empty() {
                *mine = theirs.clone();
            }
        }
        k.updated = stamp;
    });
    s.vendors.update(from, |v| v.deleted = Some(stamp));
    Ok(json!({ "into": into, "records_moved": moved }))
}

fn vendor_restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Vendor> = p.id("id")?;
    ctx.state.vendors.update(id, |v| v.deleted = None).ok_or_else(|| EngineError::not_found(format!("company {id} not found")))?;
    Ok(json!({ "id": id }))
}

fn vendor_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let q = p.line("q", 100)?.map(|s| s.to_lowercase());
    let kind = p.choice::<VendorKind>("kind")?;
    let list: Vec<&Vendor> = v
        .state
        .vendors
        .iter()
        .filter(|x| x.deleted.is_none())
        .filter(|x| kind.is_none_or(|k| x.kind == k))
        .filter(|x| q.as_ref().is_none_or(|q| x.name.to_lowercase().contains(q) || x.trade.to_lowercase().contains(q)))
        .collect();
    json(&list)
}

fn vendor_get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Vendor> = p.id("id")?;
    json(v.state.vendors.get(id).ok_or_else(|| EngineError::not_found(format!("company {id} not found")))?)
}

fn group_create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let name = p.req_line("name", text::SHORT)?;
    let members = p.ids("members")?;
    ctx.check_people(&members)?;
    let stamp = ctx.stamp();
    let id = ctx.state.groups.alloc();
    ctx.state.groups.put(DistributionGroup { id, name, project: None, members, created: stamp, updated: stamp, deleted: None });
    Ok(json!({ "id": id }))
}

fn group_update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<DistributionGroup> = p.id("id")?;
    let before =
        ctx.state.groups.get(id).filter(|g| g.deleted.is_none()).cloned().ok_or_else(|| EngineError::not_found(format!("group {id} not found")))?;
    let mut g = before.clone();
    if p.present("name") {
        g.name = p.req_line("name", text::SHORT)?;
    }
    if p.present("members") {
        let members = p.ids("members")?;
        ctx.check_people(&members)?;
        g.members = members;
    }
    if g != before {
        g.updated = ctx.stamp();
        ctx.state.groups.put(g);
    }
    Ok(json!({ "id": id }))
}

fn group_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<DistributionGroup> = p.id("id")?;
    let stamp = ctx.stamp();
    ctx.state.groups.update(id, |g| g.deleted = Some(stamp)).ok_or_else(|| EngineError::not_found(format!("group {id} not found")))?;
    Ok(json!({ "id": id }))
}

fn group_list(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    let list: Vec<&DistributionGroup> = v.state.groups.iter().filter(|g| g.deleted.is_none()).collect();
    json(&list)
}
