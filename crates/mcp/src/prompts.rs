//! Prompts: ready-made starting points an agent client can offer, each a short instruction that names the tools to use.
//! They carry no data of their own and run nothing; the tools they name apply the person's permissions as always.

use serde_json::{Value, json};

type Failure = (i64, String);

/// Writes a prompt's text given a way to read its arguments.
type Expand = fn(&dyn Fn(&str) -> String) -> String;

/// One prompt: its name, what it is for, its arguments (name, description, required) and the text it expands to.
struct Prompt {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    arguments: &'static [(&'static str, &'static str, bool)],
    text: Expand,
}

const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "project_status",
        title: "Project status briefing",
        description: "A short written status of one project: what is late, what is waiting on whom, and what needs a decision.",
        arguments: &[("project", "The project's id (project_list shows them)", true)],
        text: |a| {
            format!(
                "Write a status briefing for project {}. Use rfi_list, submittal_list, punch_list, observation_list, task_list and schedule_list \
                 (each takes project) and the budget with budget_get. Cover: overdue or aging RFIs and submittals and who holds each; punch and \
                 observation items still open, by assignee; tasks past due; schedule tasks behind or on the critical path; and the cost position \
                 (revised budget, projected cost, anything over). Finish with the three things most needing a decision this week. Quote numbers \
                 from the tools; say so when a tool returned nothing instead of guessing.",
                a("project")
            )
        },
    },
    Prompt {
        name: "draft_rfi",
        title: "Draft an RFI",
        description: "Turns a question from the field into a clear request for information, saved as a draft.",
        arguments: &[
            ("project", "The project's id", true),
            ("question", "What the field needs answered, in the asker's own words", true),
            ("sheet", "The drawing sheet or spec section it concerns, if known", false),
        ],
        text: |a| {
            format!(
                "Draft a request for information on project {} from this question: {}\n\nSheet or section: {}\n\nUse drawing_list or spec_list to check \
                 the reference and rfi_list to see whether a similar RFI already exists; if one does, say so and stop. Otherwise write a short subject, \
                 a precise question that says what is unclear and what the field proposes to do, and save it with rfi_create as a draft (do not open it). \
                 Report the RFI number.",
                a("project"),
                a("question"),
                a("sheet")
            )
        },
    },
    Prompt {
        name: "weekly_look_ahead",
        title: "Look-ahead for the week",
        description: "A plain-language look at the work starting in the coming weeks, with the conflicts to sort out first.",
        arguments: &[("project", "The project's id", true), ("weeks", "How many weeks ahead (1 to 12, default 3)", false)],
        text: |a| {
            let weeks = a("weeks");
            format!(
                "Prepare a look-ahead for project {} covering the next {} weeks. Use schedule_lookahead and schedule_loading for the work and the \
                 crews it needs, rfi_list and submittal_list for anything that could hold the work up, and meeting_list for the coordination meetings \
                 that fall in the period. Group the work by week, flag any week where a resource is over its limit or a task depends on an open RFI or \
                 submittal, and end with the questions to raise at the next meeting.",
                a("project"),
                if weeks.is_empty() { "3".to_string() } else { weeks }
            )
        },
    },
    Prompt {
        name: "closeout_check",
        title: "Closeout readiness check",
        description: "Checks what still stands between the project and closeout: open items, missing documents, unsigned papers.",
        arguments: &[("project", "The project's id", true)],
        text: |a| {
            format!(
                "Check whether project {} is ready for closeout. List what is still open with punch_list, observation_list, inspection_list and \
                 action_plan_list; look in document_list and form_list for the closeout documents and forms; check commitment_list and prime_contract_list for \
                 contracts not yet signed (signature status) and invoices not yet paid with commitment_invoice_list and owner_invoice_list. Produce a checklist \
                 grouped by who must act, with the item, its age, and the tool to find it in. Mark each group done or not done.",
                a("project")
            )
        },
    },
];

pub fn list() -> Value {
    let prompts: Vec<Value> = PROMPTS
        .iter()
        .map(|p| {
            let args: Vec<Value> = p.arguments.iter().map(|(n, d, r)| json!({ "name": n, "description": d, "required": r })).collect();
            json!({ "name": p.name, "title": p.title, "description": p.description, "arguments": args })
        })
        .collect();
    json!({ "prompts": prompts })
}

/// Expands a prompt with the arguments given. A missing required argument or an unknown prompt is an error.
pub fn get(params: &Value) -> Result<Value, Failure> {
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "prompts/get needs a `name`".to_string()))?;
    let prompt = PROMPTS.iter().find(|p| p.name == name).ok_or((-32602, format!("there is no prompt called `{name}`")))?;
    let given = params.get("arguments").and_then(Value::as_object);
    // Argument values are plain text from the client; keep them short and on one line so they can't pose as instructions.
    let value = |key: &str| -> String {
        given
            .and_then(|m| m.get(key))
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(600).collect())
            .unwrap_or_default()
    };
    for (n, _, required) in prompt.arguments {
        if *required && value(n).is_empty() {
            return Err((-32602, format!("the `{n}` argument is required")));
        }
    }
    for key in given.map(|m| m.keys()).into_iter().flatten() {
        if !prompt.arguments.iter().any(|(n, _, _)| n == key) {
            return Err((-32602, format!("`{key}` is not an argument of `{name}`")));
        }
    }
    Ok(json!({
        "description": prompt.description,
        "messages": [{ "role": "user", "content": { "type": "text", "text": (prompt.text)(&value) } }],
    }))
}
