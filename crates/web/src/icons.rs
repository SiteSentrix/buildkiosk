//! Interface icons: simple stroked shapes on a 24×24 grid, drawn for BuildKiosk and written as SVG
//! path data in code (original work, MIT OR Apache-2.0; no asset files, no third-party icon sets).

use buildkiosk_model::Tool;
use maud::{Markup, PreEscaped, html};

/// A named icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Logo,
    Home,
    Building,
    People,
    Folder,
    Sheet,
    Book,
    Camera,
    Calendar,
    CheckSquare,
    Chart,
    Question,
    Stamp,
    Send,
    Chat,
    Notebook,
    Mail,
    ListCheck,
    Clipboard,
    Eye,
    Alert,
    Target,
    Dollar,
    Shuffle,
    FileDiff,
    Contract,
    Briefcase,
    Receipt,
    Wallet,
    Hammer,
    Calculator,
    Clock,
    Truck,
    Gear,
    Shield,
    Forms,
    Plus,
    Search,
    Menu,
    SignOut,
    Download,
    Key,
    Trash,
    History,
}

impl Icon {
    /// Path data (several sub-paths per icon).
    fn paths(self) -> &'static [&'static str] {
        match self {
            Icon::Logo => &["M3 14h8v7H3z", "M13 14h8v7h-8z", "M8 7h8v7H8z", "M10.5 3.5h3"],
            Icon::Home => &["M3 11 12 4l9 7", "M5 9.5V20h14V9.5", "M10 20v-6h4v6"],
            Icon::Building => &["M4 21V5l8-3v19", "M12 8h8v13", "M7 8h2M7 12h2M7 16h2M15 12h2M15 16h2", "M2 21h20"],
            Icon::People => &["M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6z", "M3 20a6 6 0 0 1 12 0", "M16 5.5a3 3 0 0 1 0 5.5", "M18 14.5a5 5 0 0 1 3 4.5"],
            Icon::Folder => &["M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"],
            Icon::Sheet => &["M4 4h16v16H4z", "M4 9h5V4", "M14 20v-5h6", "M9 14l3-3 3 3"],
            Icon::Book => &["M5 4h10a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3z", "M5 17a3 3 0 0 1 3-3h10"],
            Icon::Camera => &["M4 8h3l2-3h6l2 3h3v11H4z", "M12 16.5a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"],
            Icon::Calendar => &["M4 6h16v14H4z", "M4 10h16", "M8 3v4M16 3v4"],
            Icon::CheckSquare => &["M4 4h16v16H4z", "M8 12l3 3 5-6"],
            Icon::Chart => &["M5 20V11", "M11 20V5", "M17 20v-6", "M3 20h18"],
            Icon::Question => &["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6", "M12 17h.01"],
            Icon::Stamp => &["M8 21h8", "M5 17h14v-3H5z", "M9 14V9a3 3 0 1 1 6 0v5"],
            Icon::Send => &["M21 3 10 14", "M21 3l-7 18-4-7-7-4z"],
            Icon::Chat => &["M4 5h12v8H8l-4 3z", "M16 9h4v8l-3-2h-5v-2"],
            Icon::Notebook => &["M6 3h12v18H6z", "M9 7h6M9 11h6M9 15h4", "M4 6h2M4 12h2M4 18h2"],
            Icon::Mail => &["M3 6h18v12H3z", "M3 7l9 6 9-6"],
            Icon::ListCheck => &["M10 6h10M10 12h10M10 18h10", "M3 6l1.5 1.5L7 5", "M3 12l1.5 1.5L7 11", "M3 18l1.5 1.5L7 17"],
            Icon::Clipboard => &["M8 4h8v3H8z", "M6 5H5v16h14V5h-1", "M9 13l2 2 4-4"],
            Icon::Eye => &["M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12z", "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"],
            Icon::Alert => &["M12 3 2 20h20z", "M12 10v4", "M12 17h.01"],
            Icon::Target => &["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8z", "M12 12h.01"],
            Icon::Dollar => &["M12 3v18", "M16.5 7.5C15.5 6 14 5.5 12 5.5c-2.5 0-4 1.2-4 3s1.5 2.6 4 3.1 4 1.4 4 3.2-1.6 3-4 3c-2.2 0-3.8-.7-4.6-2"],
            Icon::Shuffle => &["M3 7h4l10 10h4", "M3 17h4l3-3", "M14 10l3-3h4", "M19 5l2 2-2 2", "M19 15l2 2-2 2"],
            Icon::FileDiff => &["M6 3h9l4 4v14H6z", "M10 11h5M12.5 8.5v5", "M10 17h5"],
            Icon::Contract => &["M6 3h12v18H6z", "M9 8h6M9 12h6", "M9 17c1-1.5 2-1.5 3 0s2 1.5 3 0"],
            Icon::Briefcase => &["M3 8h18v12H3z", "M8 8V5h8v3", "M3 13h18"],
            Icon::Receipt => &["M6 3h12v18l-3-2-3 2-3-2-3 2z", "M9 8h6M9 12h6"],
            Icon::Wallet => &["M3 7h16a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H3z", "M3 7V5h13", "M16 13.5h2"],
            Icon::Hammer => &["M13 4l7 7-3 3-7-7z", "M11.5 8.5 3 17l3 3 8.5-8.5"],
            Icon::Calculator => &["M6 3h12v18H6z", "M9 7h6", "M9 12h.01M12 12h.01M15 12h.01M9 16h.01M12 16h.01M15 16h.01"],
            Icon::Clock => &["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M12 7v5l3 2"],
            Icon::Truck => &["M3 7h11v9H3z", "M14 10h4l3 3v3h-7", "M7 19a2 2 0 1 0 0-4 2 2 0 0 0 0 4z", "M17 19a2 2 0 1 0 0-4 2 2 0 0 0 0 4z"],
            Icon::Gear => &[
                "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
                "M19 12h2M3 12h2M12 3v2M12 19v2M17 7l1.4-1.4M5.6 18.4 7 17M17 17l1.4 1.4M5.6 5.6 7 7",
            ],
            Icon::Shield => &["M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z", "M9 12l2 2 4-4"],
            Icon::Forms => &["M6 3h12v18H6z", "M9 8h6M9 12h6M9 16h3"],
            Icon::Plus => &["M12 5v14M5 12h14"],
            Icon::Search => &["M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z", "M21 21l-4.5-4.5"],
            Icon::Menu => &["M4 7h16M4 12h16M4 17h16"],
            Icon::SignOut => &["M10 4H5v16h5", "M14 8l4 4-4 4", "M18 12H9"],
            Icon::Download => &["M12 4v11", "M7 10l5 5 5-5", "M5 20h14"],
            Icon::Key => &["M8 15a4 4 0 1 0 0-8 4 4 0 0 0 0 8z", "M11.5 11H21v3", "M17 11v3"],
            Icon::Trash => &["M4 7h16", "M9 7V4h6v3", "M6 7l1 13h10l1-13"],
            Icon::History => &["M3 12a9 9 0 1 0 3-6.7", "M3 4v4h4", "M12 8v4l3 2"],
        }
    }

    pub fn svg(self) -> Markup {
        self.svg_class("icon")
    }

    pub fn svg_class(self, class: &str) -> Markup {
        let mut d = String::new();
        for p in self.paths() {
            d.push_str(p);
            d.push(' ');
        }
        html! {
            svg class=(class) viewBox="0 0 24 24" aria-hidden="true" focusable="false" {
                path d=(d.trim_end()) {}
            }
        }
    }

    /// The favicon: the logo mark as a standalone SVG document.
    pub fn favicon() -> String {
        let d: String = Icon::Logo.paths().join(" ");
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="5" fill="#1f5fbf"/><path d="{d}" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##
        )
    }
}

/// The icon shown for a tool.
pub fn tool_icon(tool: Tool) -> Icon {
    use Tool::*;
    match tool {
        Portfolio => Icon::Building,
        Directory | ProjectDirectory => Icon::People,
        CompanyAdmin | ProjectAdmin => Icon::Gear,
        CompanyReports | Reports => Icon::Chart,
        Equipment => Icon::Truck,
        Prequalification => Icon::Shield,
        ProjectHome => Icon::Home,
        Documents => Icon::Folder,
        Drawings => Icon::Sheet,
        Specifications => Icon::Book,
        Photos => Icon::Camera,
        Schedule => Icon::Calendar,
        Tasks => Icon::CheckSquare,
        Rfis => Icon::Question,
        Submittals => Icon::Stamp,
        Transmittals => Icon::Send,
        Meetings => Icon::Chat,
        DailyLog => Icon::Notebook,
        Correspondence => Icon::Mail,
        Forms => Icon::Forms,
        PunchList => Icon::ListCheck,
        Inspections => Icon::Clipboard,
        Observations => Icon::Eye,
        Incidents => Icon::Alert,
        ActionPlans => Icon::Target,
        Budget => Icon::Dollar,
        ChangeEvents => Icon::Shuffle,
        ChangeOrders => Icon::FileDiff,
        Commitments => Icon::Contract,
        PrimeContracts => Icon::Briefcase,
        Invoicing => Icon::Receipt,
        DirectCosts => Icon::Wallet,
        Bidding => Icon::Hammer,
        Estimating => Icon::Calculator,
        Timesheets => Icon::Clock,
    }
}

/// Raw SVG markup for places that need a string.
pub fn raw(icon: Icon) -> PreEscaped<String> {
    icon.svg()
}
