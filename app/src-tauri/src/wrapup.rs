//! After a meeting is written up: telling the user, wherever they are. In
//! Quill's window, the meeting and a toast say it (the page handles that);
//! on the wrap-up card, the card does; otherwise a system notification.

use quill_core::processing::{StepKind, StepStatus};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::commands::lock;
use crate::pipeline::{Progress, Stage};

/// A write-up finished (or failed).
pub fn finished(app: &AppHandle, progress: &Progress) {
    let Some(report) = &progress.report else {
        return;
    };
    // A write-up (transcribing or notes), not re-splitting speakers.
    let write_up = report
        .steps
        .iter()
        .any(|s| s.kind != StepKind::Speakers && s.status != StepStatus::Skipped);
    if !write_up || !lock(&app.state::<AppState>().settings).notify_notes_ready {
        return;
    }
    if crate::live_notes::wrapping(app).as_deref() == Some(progress.meeting_id.as_str()) {
        return;
    }
    let in_front = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
    if in_front {
        return;
    }
    let title = progress
        .title
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or("your meeting");
    let (heading, body) = match progress.stage {
        Stage::Done => {
            let body = match &report.outcome {
                Some(o) if o.notes => format!(
                    "{} action {} · {} {}",
                    o.action_items,
                    if o.action_items == 1 { "item" } else { "items" },
                    o.decisions,
                    if o.decisions == 1 {
                        "decision"
                    } else {
                        "decisions"
                    },
                ),
                _ => "The transcript is ready.".to_string(),
            };
            (format!("Notes ready: {title}"), body)
        }
        Stage::Failed => {
            let message = progress
                .message
                .as_deref()
                .unwrap_or("Something went wrong.");
            // The first sentence: the rest is for the meeting page.
            let first = message
                .split_inclusive(". ")
                .next()
                .unwrap_or(message)
                .trim();
            (format!("Couldn't finish {title}"), first.to_string())
        }
        _ => return,
    };
    crate::notify::meeting(app, &progress.meeting_id, &heading, &body);
}
