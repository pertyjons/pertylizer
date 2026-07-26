//! Canonical human-readable labels for domain enums.
//!
//! A small enum that the user picks from is otherwise written down twice: once
//! as the picker's `(variant, "Label")` table and once as the read-back label
//! (tooltip, tracker cell, node title). The two copies drift — that is how the
//! Note Grid ended up calling the same node both "Euclidean Generator" and
//! "Euclidean". Implementing [`DisplayName`] puts the label and the presentation
//! order on the enum itself, so every surface renders it from one definition.

/// A domain enum that owns its user-facing labels and presentation order.
///
/// Implement this for any `Copy` enum the GUI offers as a choice, then render it
/// with the `enum_combo_all` widget instead of hand-listing the variants. Keep
/// the label here — never in the view — so pickers and read-back text can't
/// disagree.
pub trait DisplayName: Copy + PartialEq + 'static {
    /// Every variant, in the order a picker should present them.
    const ALL: &'static [Self];

    /// The user-facing label for this variant.
    fn display_name(self) -> &'static str;
}
