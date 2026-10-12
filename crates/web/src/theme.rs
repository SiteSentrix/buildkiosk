//! Design tokens. Every colour in the interface comes from here (AGENTS.md › The UI is thin); the
//! stylesheet turns them into CSS custom properties for the light and dark schemes.

/// One colour scheme.
pub struct Tokens {
    pub bg: &'static str,
    pub surface: &'static str,
    pub surface_2: &'static str,
    pub border: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    pub primary: &'static str,
    pub primary_ink: &'static str,
    pub primary_soft: &'static str,
    pub accent: &'static str,
    pub ok: &'static str,
    pub ok_soft: &'static str,
    pub warn: &'static str,
    pub warn_soft: &'static str,
    pub danger: &'static str,
    pub danger_soft: &'static str,
    pub violet: &'static str,
    pub violet_soft: &'static str,
    pub topbar: &'static str,
    pub topbar_ink: &'static str,
    pub focus: &'static str,
}

pub const LIGHT: Tokens = Tokens {
    bg: "#f5f6f8",
    surface: "#ffffff",
    surface_2: "#f0f2f5",
    border: "#dfe3e8",
    text: "#18212c",
    muted: "#5a6878",
    primary: "#1f5fbf",
    primary_ink: "#ffffff",
    primary_soft: "#e7effb",
    accent: "#0d7a6e",
    ok: "#1b7a3d",
    ok_soft: "#e6f5eb",
    warn: "#9a5800",
    warn_soft: "#fff3dc",
    danger: "#c22d2d",
    danger_soft: "#fcebeb",
    violet: "#6b3fc4",
    violet_soft: "#f0eafc",
    topbar: "#132030",
    topbar_ink: "#e8eef6",
    focus: "#3b7ff0",
};

pub const DARK: Tokens = Tokens {
    bg: "#0e1319",
    surface: "#151c25",
    surface_2: "#1b2430",
    border: "#2a3543",
    text: "#e5eaf0",
    muted: "#98a5b4",
    primary: "#6aa2f2",
    primary_ink: "#0c1520",
    primary_soft: "#1a2a42",
    accent: "#3cc2b0",
    ok: "#5ccf86",
    ok_soft: "#14301f",
    warn: "#f0b14a",
    warn_soft: "#36280f",
    danger: "#f07272",
    danger_soft: "#3a1717",
    violet: "#b294f5",
    violet_soft: "#2a2042",
    topbar: "#0a1018",
    topbar_ink: "#dfe7f1",
    focus: "#6aa2f2",
};

impl Tokens {
    /// The tokens as CSS custom properties.
    pub fn css_vars(&self) -> String {
        let vars = [
            ("bg", self.bg),
            ("surface", self.surface),
            ("surface-2", self.surface_2),
            ("border", self.border),
            ("text", self.text),
            ("muted", self.muted),
            ("primary", self.primary),
            ("primary-ink", self.primary_ink),
            ("primary-soft", self.primary_soft),
            ("accent", self.accent),
            ("ok", self.ok),
            ("ok-soft", self.ok_soft),
            ("warn", self.warn),
            ("warn-soft", self.warn_soft),
            ("danger", self.danger),
            ("danger-soft", self.danger_soft),
            ("violet", self.violet),
            ("violet-soft", self.violet_soft),
            ("topbar", self.topbar),
            ("topbar-ink", self.topbar_ink),
            ("focus", self.focus),
        ];
        vars.iter().map(|(k, v)| format!("--{k}:{v};")).collect()
    }
}
