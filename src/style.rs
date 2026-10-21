//! Typed style declarations compiled to static CSS.

/// Element arguments that take style values rather than expressions, e.g. `column(gap: 20px)`.
pub fn is_style_arg(name: &str) -> bool {
    matches!(name, "gap" | "align" | "justify" | "padding" | "wrap" | "columns" | "width" | "height" | "grow")
}
