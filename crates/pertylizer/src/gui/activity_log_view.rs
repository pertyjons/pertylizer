//! Self-contained activity-log console.
//!
//! A pure view over a shared [`ActivityLog`] plus its own small UI state
//! ([`ActivityLogViewState`], owned by the app). Built free-standing — a free
//! function over `(&ActivityLog, &mut ActivityLogViewState)` — so it can later
//! be promoted from the Home right panel to its own tab or a bottom console
//! with a one-line mount and no data-model change.
//!
//! Filtering happens here, over a snapshot of the buffer, so the buffer keeps
//! full debug-level detail and the Debug toggle is instant. The snapshot is
//! only refreshed when the log's [generation](ActivityLog::generation) changes,
//! so an idle frame does no work.

use eframe::egui::{self, Color32, RichText};
use egui_remixicon::icons as ri;

use crate::activity_log::{ActivityLog, LogEntry, LogLevel};
use crate::gui::list_panel;
use crate::gui::theme::{Theme, theme};
use crate::gui::widgets::{icon_button, toggle_button_colored};

/// How often to nudge a repaint while the console is visible. Tracing events
/// arrive from other threads and don't wake egui, so without this the log would
/// only refresh on user interaction. ~4 Hz keeps it live without a 60 fps
/// repaint storm; the snapshot is generation-gated, so idle ticks stay cheap.
const REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// An action the console asks the host app to perform. Returned from [`show`]
/// because it needs app-level facilities (the file dialog) the view can't reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityLogAction {
    /// Open a save dialog to export the current log to a file.
    Export,
}

/// UI state for the activity-log console. Owned by the app and passed in each
/// frame; the buffer itself lives in the shared [`ActivityLog`].
pub struct ActivityLogViewState {
    /// Severity floor when [`show_debug`](Self::show_debug) is off. Entries at
    /// or above this severity (`Error`/`Warn`/`Info`) are shown.
    pub min_level: LogLevel,
    /// When on, the floor drops to include `Debug` (and `Trace`) entries.
    pub show_debug: bool,
    /// Case-insensitive substring filter over target + message. Empty = no
    /// filter.
    pub text_filter: String,
    /// Stick the scroll view to the bottom as new entries arrive.
    pub autoscroll: bool,
    /// Reused snapshot buffer — avoids a per-frame `Vec` allocation.
    scratch: Vec<LogEntry>,
    /// Last log generation copied into `scratch`; re-snapshot only on change.
    last_generation: u64,
    /// Whether `scratch` has ever been filled.
    has_snapshot: bool,
    /// [`LogEntry::seq`] of the row whose fields are expanded, if any. Keyed by
    /// the stable seq (not a snapshot index) so a row stays expanded as new
    /// entries arrive; it simply disappears once evicted from the buffer.
    expanded: Option<u64>,
}

impl Default for ActivityLogViewState {
    fn default() -> Self {
        Self {
            min_level: LogLevel::Info,
            show_debug: false,
            text_filter: String::new(),
            autoscroll: true,
            scratch: Vec::new(),
            last_generation: 0,
            has_snapshot: false,
            expanded: None,
        }
    }
}

/// The effective severity floor: `Trace` (show everything) when the Debug
/// toggle is on, otherwise the chip-selected `min_level`.
fn effective_floor(show_debug: bool, min_level: LogLevel) -> LogLevel {
    if show_debug {
        LogLevel::Trace
    } else {
        min_level
    }
}

/// ASCII-case-insensitive substring test that allocates nothing (unlike
/// `haystack.to_ascii_lowercase().contains(needle)`), so it stays cheap when
/// run over every buffered entry each repaint.
fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    let (hay, ndl) = (haystack.as_bytes(), needle.as_bytes());
    if ndl.is_empty() {
        return true;
    }
    if ndl.len() > hay.len() {
        return false;
    }
    hay.windows(ndl.len())
        .any(|w| w.iter().zip(ndl).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

/// Whether `entry` passes the level `floor` and the `needle` text filter (empty
/// `needle` = no text filter, matched case-insensitively). `LogLevel` orders
/// Error (most severe) < … < Trace, so an entry shows when its level is at or
/// above (numerically ≤) the floor.
fn entry_passes(entry: &LogEntry, floor: LogLevel, needle: &str) -> bool {
    if entry.level > floor {
        return false;
    }
    if needle.is_empty() {
        return true;
    }
    contains_ascii_ci(&entry.target, needle) || contains_ascii_ci(&entry.message, needle)
}

/// Console color for a severity level.
fn level_color(level: LogLevel, t: &Theme) -> Color32 {
    match level {
        LogLevel::Error => t.colors.accent_red,
        LogLevel::Warn => t.colors.accent_orange,
        LogLevel::Info => t.colors.text_secondary,
        LogLevel::Debug => t.colors.text_dim,
        LogLevel::Trace => t.colors.text_dim,
    }
}

/// Format a capture time as a local `HH:MM:SS` time-of-day.
fn format_time(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .format("%H:%M:%S")
        .to_string()
}

/// The one-line rendering of an entry, shared by the console rows and the
/// export: `HH:MM:SS  LEVEL  target  message`.
fn format_line(entry: &LogEntry) -> String {
    format!(
        "{}  {:<5}  {}  {}",
        format_time(entry.time),
        entry.level.tag(),
        entry.target,
        entry.message,
    )
}

/// Render the whole log to plain text for export: one entry per line as
/// [`format_line`], with any structured fields appended as ` key=value`.
#[must_use]
pub fn format_export(log: &ActivityLog) -> String {
    let mut snapshot = Vec::new();
    log.snapshot_into(&mut snapshot);
    let mut out = String::new();
    for entry in &snapshot {
        out.push_str(&format_line(entry));
        for (key, value) in &entry.fields {
            out.push_str(&format!("  {key}={value}"));
        }
        out.push('\n');
    }
    out
}

/// Render the activity-log console into `ui`, returning any action the host app
/// must carry out (currently only [`ActivityLogAction::Export`]).
pub fn show(
    ui: &mut egui::Ui,
    log: &ActivityLog,
    state: &mut ActivityLogViewState,
) -> Option<ActivityLogAction> {
    let t = theme();
    let mut action = None;

    // ── Header row ─────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}  Activity", ri::TERMINAL_BOX_LINE))
                .size(t.fonts.size_normal)
                .strong()
                .color(t.colors.accent_cyan),
        );

        ui.separator();

        // Severity floor chips.
        for (label, level) in [
            ("Info", LogLevel::Info),
            ("Warn", LogLevel::Warn),
            ("Error", LogLevel::Error),
        ] {
            let active = !state.show_debug && state.min_level == level;
            if toggle_button_colored(ui, label, active, level_color(level, &t))
                .on_hover_text(format!("Show {label} and more severe"))
                .clicked()
            {
                state.min_level = level;
                state.show_debug = false;
            }
        }

        // Debug toggle — drops the floor to include Debug/Trace.
        if toggle_button_colored(ui, "Debug", state.show_debug, t.colors.accent_yellow)
            .on_hover_text("Include Debug/Trace detail")
            .clicked()
        {
            state.show_debug = !state.show_debug;
        }

        // Right-aligned controls: Clear · Export · autoscroll toggle
        // (right-to-left, so the first added lands rightmost).
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, ri::DELETE_BIN_LINE, t.colors.text_dim, "Clear the log").clicked() {
                log.clear();
                state.expanded = None;
            }
            if icon_button(
                ui,
                ri::DOWNLOAD_LINE,
                t.colors.text_dim,
                "Export the log to a file",
            )
            .clicked()
            {
                action = Some(ActivityLogAction::Export);
            }
            let scroll_color = if state.autoscroll {
                t.colors.accent_primary
            } else {
                t.colors.text_dim
            };
            let scroll_tip = if state.autoscroll {
                "Autoscroll on — click to pause"
            } else {
                "Autoscroll paused — click to follow"
            };
            if icon_button(ui, ri::ARROW_DOWN_LINE, scroll_color, scroll_tip).clicked() {
                state.autoscroll = !state.autoscroll;
            }
        });
    });

    list_panel::search_box(ui, &mut state.text_filter);

    ui.separator();

    // Tracing events arrive off-thread and don't wake egui; keep the console
    // live while it's on screen.
    ui.ctx().request_repaint_after(REPAINT_INTERVAL);

    // ── Refresh the snapshot only when the buffer changed ──────────────────
    let generation = log.generation();
    if !state.has_snapshot || generation != state.last_generation {
        log.snapshot_into(&mut state.scratch);
        state.last_generation = generation;
        state.has_snapshot = true;
    }

    if state.scratch.is_empty() {
        ui.add_space(t.spacing.md);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No activity yet").color(t.colors.text_dim));
        });
        return action;
    }

    // ── Body: one monospace row per visible entry ──────────────────────────
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(state.autoscroll)
        .show(ui, |ui| {
            // Disjoint field borrows: read the snapshot + filters immutably
            // while toggling the expanded row mutably.
            let scratch = &state.scratch;
            let expanded = &mut state.expanded;
            let floor = effective_floor(state.show_debug, state.min_level);
            let needle = state.text_filter.as_str();

            for entry in scratch {
                if !entry_passes(entry, floor, needle) {
                    continue;
                }

                let color = level_color(entry.level, &t);
                let resp = ui.add(
                    egui::Label::new(RichText::new(format_line(entry)).monospace().color(color))
                        .sense(egui::Sense::click())
                        .wrap(),
                );
                if resp.clicked() {
                    *expanded = if *expanded == Some(entry.seq) {
                        None
                    } else {
                        Some(entry.seq)
                    };
                }

                // Expanded: show the structured fields inline, indented.
                if *expanded == Some(entry.seq) && !entry.fields.is_empty() {
                    for (key, value) in &entry.fields {
                        ui.label(
                            RichText::new(format!("        {key} = {value}"))
                                .monospace()
                                .size(t.fonts.size_small)
                                .color(t.colors.text_dim),
                        );
                    }
                }
            }
        });

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(level: LogLevel, target: &str, message: &str) -> LogEntry {
        LogEntry {
            seq: 0,
            time: std::time::UNIX_EPOCH,
            level,
            target: target.to_string(),
            message: message.to_string(),
            fields: Vec::new(),
        }
    }

    /// Filter as the view does: floor from (show_debug, min_level), needle
    /// lowercased.
    fn passes(st: &ActivityLogViewState, entry: &LogEntry) -> bool {
        let floor = effective_floor(st.show_debug, st.min_level);
        entry_passes(entry, floor, &st.text_filter.to_ascii_lowercase())
    }

    #[test]
    fn floor_hides_below_min_level() {
        let mut st = ActivityLogViewState::default(); // Info floor
        assert!(passes(&st, &mk(LogLevel::Error, "t", "m")));
        assert!(passes(&st, &mk(LogLevel::Info, "t", "m")));
        assert!(!passes(&st, &mk(LogLevel::Debug, "t", "m")));
        st.show_debug = true;
        assert!(passes(&st, &mk(LogLevel::Debug, "t", "m")));
        assert!(passes(&st, &mk(LogLevel::Trace, "t", "m")));
    }

    #[test]
    fn warn_floor_hides_info() {
        let st = ActivityLogViewState {
            min_level: LogLevel::Warn,
            ..Default::default()
        };
        assert!(passes(&st, &mk(LogLevel::Warn, "t", "m")));
        assert!(!passes(&st, &mk(LogLevel::Info, "t", "m")));
    }

    #[test]
    fn text_filter_matches_target_or_message_case_insensitively() {
        let st = ActivityLogViewState {
            text_filter: "MCP".to_string(),
            ..Default::default()
        };
        assert!(passes(&st, &mk(LogLevel::Info, "synth_mcp::server", "hi")));
        assert!(passes(&st, &mk(LogLevel::Info, "other", "an mcp thing")));
        assert!(!passes(&st, &mk(LogLevel::Info, "other", "nothing here")));
    }

    #[test]
    fn contains_ascii_ci_matches_regardless_of_case() {
        assert!(contains_ascii_ci("synth_MCP::server", "mcp"));
        assert!(contains_ascii_ci("hello", "HELLO"));
        assert!(contains_ascii_ci("anything", "")); // empty needle
        assert!(!contains_ascii_ci("short", "longer-than-haystack"));
        assert!(!contains_ascii_ci("abc", "xyz"));
    }

    #[test]
    fn format_export_renders_lines_with_fields() {
        let log = ActivityLog::new();
        let mut e = mk(LogLevel::Info, "synth_mcp::call", "MCP tool call");
        e.fields = vec![("tool".to_string(), "set_parameter".to_string())];
        log.push(e);
        let text = format_export(&log);
        assert!(text.contains("INFO"));
        assert!(text.contains("synth_mcp::call"));
        assert!(text.contains("MCP tool call"));
        assert!(text.contains("tool=set_parameter"));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn format_time_is_hh_mm_ss_shape() {
        // The exact value is timezone-dependent (local time), so assert the
        // `HH:MM:SS` shape rather than a fixed instant.
        let out = format_time(std::time::UNIX_EPOCH + std::time::Duration::from_secs(3661));
        assert_eq!(out.len(), 8, "got {out:?}");
        let bytes = out.as_bytes();
        assert_eq!(bytes[2], b':');
        assert_eq!(bytes[5], b':');
        assert!(
            out.chars().enumerate().all(|(i, c)| {
                if i == 2 || i == 5 {
                    c == ':'
                } else {
                    c.is_ascii_digit()
                }
            }),
            "got {out:?}"
        );
    }
}
