//! Puts back what a person typed into a form whose save was refused.
//!
//! The server keeps the refused submission briefly; when the page the person is sent back to is
//! rendered, this walks the finished HTML and sets the fields of the form that was submitted
//! (matched by its `action`) to the submitted values, so nothing has to be retyped.

/// One attribute as written in the source: its name and, if it has one, its raw (escaped) value.
struct Attr {
    name: String,
    value: Option<String>,
}

struct Tag {
    /// Lower-case element name, without a leading `/` for closing tags.
    name: String,
    closing: bool,
    attrs: Vec<Attr>,
    /// Index just past the `>`.
    end: usize,
}

fn parse_tag(s: &str, at: usize) -> Option<Tag> {
    let bytes = s.as_bytes();
    let mut i = at + 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    while bytes.get(i).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-') {
        i += 1;
    }
    if i == name_start {
        return None; // a comment, a doctype or a stray `<`
    }
    let name = s.get(name_start..i)?.to_ascii_lowercase();
    let mut attrs = Vec::new();
    loop {
        while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace() || *b == b'/') {
            i += 1;
        }
        match bytes.get(i) {
            None => return None,
            Some(b'>') => return Some(Tag { name, closing, attrs, end: i + 1 }),
            _ => {}
        }
        let a_start = i;
        while bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace() && *b != b'=' && *b != b'>' && *b != b'/') {
            i += 1;
        }
        let a_name = s.get(a_start..i)?.to_ascii_lowercase();
        let mut value = None;
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            match bytes.get(i) {
                Some(q @ (b'"' | b'\'')) => {
                    let v_start = i + 1;
                    let rel = s.get(v_start..)?.find(char::from(*q))?;
                    value = Some(s.get(v_start..v_start + rel)?.to_string());
                    i = v_start + rel + 1;
                }
                _ => {
                    let v_start = i;
                    while bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>') {
                        i += 1;
                    }
                    value = Some(s.get(v_start..i)?.to_string());
                }
            }
        }
        if a_name.is_empty() {
            return None;
        }
        attrs.push(Attr { name: a_name, value });
    }
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn attr<'a>(t: &'a Tag, name: &str) -> Option<&'a Attr> {
    t.attrs.iter().find(|a| a.name == name)
}

fn attr_text(t: &Tag, name: &str) -> Option<String> {
    attr(t, name).and_then(|a| a.value.as_deref()).map(unescape)
}

/// The tag written out again, with `set` replacing or adding attributes (`None` removes one).
fn rebuild(t: &Tag, set: &[(&str, Option<String>)]) -> String {
    let mut out = format!("<{}", t.name);
    for a in &t.attrs {
        if set.iter().any(|(n, _)| *n == a.name) {
            continue;
        }
        out.push(' ');
        out.push_str(&a.name);
        if let Some(v) = &a.value {
            out.push_str(&format!("=\"{}\"", v.replace('"', "&quot;")));
        }
    }
    for (n, v) in set {
        if let Some(v) = v {
            out.push(' ');
            out.push_str(n);
            if !v.is_empty() || *n == "value" {
                out.push_str(&format!("=\"{}\"", escape(v)));
            }
        }
    }
    out.push('>');
    out
}

fn values_of<'a>(values: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    values.iter().filter(|(k, _)| k == name).map(|(_, v)| v.as_str()).collect()
}

/// Sets the fields of the form whose `action` is `action` to `values` (the submitted name/value
/// pairs, in order). Other forms and everything outside forms are left as they are.
pub fn restore_form(html: &str, action: &str, values: &[(String, String)]) -> String {
    let mut out = String::with_capacity(html.len() + 256);
    let mut pos = 0;
    let mut in_form = false;
    let mut select: Option<String> = None;
    while let Some(rel) = html.get(pos..).and_then(|r| r.find('<')) {
        let lt = pos + rel;
        out.push_str(html.get(pos..lt).unwrap_or_default());
        let Some(tag) = parse_tag(html, lt) else {
            out.push('<');
            pos = lt + 1;
            continue;
        };
        let raw = html.get(lt..tag.end).unwrap_or_default();
        pos = tag.end;
        if tag.name == "form" {
            if tag.closing {
                in_form = false;
                select = None;
            } else {
                in_form = attr_text(&tag, "action").as_deref() == Some(action);
            }
            out.push_str(raw);
            continue;
        }
        if !in_form || tag.closing && tag.name != "select" {
            out.push_str(raw);
            continue;
        }
        match tag.name.as_str() {
            "input" => {
                let kind = attr_text(&tag, "type").unwrap_or_else(|| "text".into()).to_ascii_lowercase();
                let name = attr_text(&tag, "name").unwrap_or_default();
                if name.is_empty() || matches!(kind.as_str(), "hidden" | "file" | "submit" | "button" | "image" | "password" | "reset") {
                    out.push_str(raw);
                } else if matches!(kind.as_str(), "checkbox" | "radio") {
                    let mine = attr_text(&tag, "value").unwrap_or_else(|| "on".into());
                    let on = values_of(values, &name).contains(&mine.as_str());
                    out.push_str(&rebuild(&tag, &[("checked", if on { Some(String::new()) } else { None })]));
                } else if let Some(v) = values_of(values, &name).last() {
                    out.push_str(&rebuild(&tag, &[("value", Some((*v).to_string()))]));
                } else {
                    out.push_str(raw);
                }
            }
            "textarea" if !tag.closing => {
                let name = attr_text(&tag, "name").unwrap_or_default();
                let close = html.get(pos..).and_then(|r| r.find("</textarea")).map(|r| pos + r);
                match (values_of(values, &name).last(), close) {
                    (Some(v), Some(close)) if !name.is_empty() => {
                        out.push_str(raw);
                        out.push_str(&escape(v));
                        pos = close;
                    }
                    _ => out.push_str(raw),
                }
            }
            "select" => {
                if tag.closing {
                    select = None;
                } else {
                    select = attr_text(&tag, "name").filter(|n| !n.is_empty());
                }
                out.push_str(raw);
            }
            "option" if !tag.closing => match &select {
                Some(name) if values.iter().any(|(k, _)| k == name) => {
                    let mine = attr_text(&tag, "value").unwrap_or_default();
                    let on = values_of(values, name).contains(&mine.as_str());
                    out.push_str(&rebuild(&tag, &[("selected", if on { Some(String::new()) } else { None })]));
                }
                _ => out.push_str(raw),
            },
            _ => out.push_str(raw),
        }
    }
    out.push_str(html.get(pos..).unwrap_or_default());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect()
    }

    const PAGE: &str = concat!(
        r#"<form method="get" action="/search"><input type="search" name="q" value="old"></form>"#,
        r#"<form method="post" action="/do/rfi.create"><input type="hidden" name="_csrf" value="tok">"#,
        r#"<input type="text" name="subject" value="" required><input type="date" name="due">"#,
        r#"<textarea name="question" class="short">old text</textarea>"#,
        r#"<select name="priority"><option value="low" selected>Low</option><option value="high">High</option></select>"#,
        r#"<input type="checkbox" name="assignees[]" value="1" checked><input type="checkbox" name="assignees[]" value="2">"#,
        r#"<select name="who[]" multiple><option value="a">A</option><option value="b">B</option></select>"#,
        r#"<input type="password" name="password" value=""><button type="submit">Save</button></form>"#,
    );

    #[test]
    fn sets_fields_of_the_matching_form_only() {
        let typed = v(&[
            ("_csrf", "other"),
            ("subject", "A \"quoted\" <b> & more"),
            ("due", "2026-10-30"),
            ("question", "Which hinge?\nSecond line <script>"),
            ("priority", "high"),
            ("assignees[]", "2"),
            ("who[]", "a"),
            ("who[]", "b"),
            ("password", "secret"),
            ("q", "changed"),
        ]);
        let out = restore_form(PAGE, "/do/rfi.create", &typed);
        assert!(out.contains(r#"name="subject" required value="A &quot;quoted&quot; &lt;b&gt; &amp; more""#), "{out}");
        assert!(out.contains(r#"name="due" value="2026-10-30""#));
        assert!(out.contains("Which hinge?\nSecond line &lt;script&gt;</textarea>"));
        assert!(out.contains(r#"<option value="low">Low</option>"#) && out.contains(r#"<option value="high" selected>High</option>"#));
        assert!(out.contains(r#"name="assignees[]" value="1">"#) && out.contains(r#"value="2" checked>"#));
        assert!(out.contains(r#"<option value="a" selected>"#) && out.contains(r#"<option value="b" selected>"#));
        // The other form, the CSRF token and the password are untouched.
        assert!(
            out.contains(r#"name="q" value="old""#) && out.contains(r#"name="_csrf" value="tok""#) && out.contains(r#"name="password" value="""#)
        );
        // Nothing else changed: same length apart from the edits.
        assert!(out.starts_with(r#"<form method="get" action="/search">"#) && out.ends_with("</button></form>"));
    }

    #[test]
    fn other_actions_and_junk_are_left_alone() {
        assert_eq!(restore_form(PAGE, "/do/other", &v(&[("subject", "x")])), PAGE);
        let weird = "a < b <!-- c --> <p>1 &lt; 2</p><form action=\"/f\"><input name=n></form>";
        let out = restore_form(weird, "/f", &v(&[("n", "7")]));
        assert!(out.starts_with("a < b <!-- c --> <p>1 &lt; 2</p>") && out.contains(r#"<input name="n" value="7">"#), "{out}");
        assert_eq!(restore_form("", "/f", &[]), "");
        let cut = "<form action=\"/f\"><input name=x";
        assert_eq!(restore_form(cut, "/f", &v(&[("x", "1")])), cut, "an unfinished tag is kept as it was");
    }
}
