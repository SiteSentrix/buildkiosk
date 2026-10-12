//! The notifications page.

use maud::{Markup, html};

use crate::icons::Icon;
use crate::layout::{self, Nav, Req, button_form, empty};

pub fn page(req: &Req<'_>) -> Markup {
    let state = req.engine.state();
    let mut mine: Vec<_> = state.notifications.iter().filter(|n| n.person == req.person).collect();
    mine.reverse();
    mine.truncate(100);
    let unread = mine.iter().filter(|n| !n.read).count();
    let body = html! {
        div class="page-head" {
            div { h1 { "Notifications" } div class="sub" { (unread) " unread" } }
            div class="actions" { @if unread > 0 { (button_form(req, "notification.read", "/notifications", &[], "Mark all as read", "")) } }
        }
        @if mine.is_empty() {
            div class="card" { (empty(Icon::Mail, "Nothing yet", "You'll be told here when something needs you, like a submittal waiting for your review.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    tbody {
                        @for n in &mine {
                            tr {
                                td { @if !n.read { span class="pill blue" { "New" } } }
                                td { a class="row-link" href=(n.link) { (n.text) } }
                                td class="nowrap muted" { (req.when(n.created.at)) }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(req, "Notifications", Nav::Notifications, &[("Notifications", None)], body)
}
