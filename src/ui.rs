//! Small, repeated fragments that must look the same everywhere.
//!
//! Askama 0.12 has no template macros, and an `{% include %}` is compiled as a
//! separate block that cannot take parameters — so a *per-row* fragment (the
//! status badge in a list) cannot be a partial that reads the loop variable.
//! Those fragments are built here instead and rendered with `|safe`; the markup
//! and the colour-by-status rule therefore live in exactly one place.
//!
//! Fragments that *can* be a parameterless partial (the empty state, the
//! form-level error banner) stay in `templates/partials/` and read a field off
//! the including template struct.

/// The Tailwind colour classes for a content status. One list, used by every
/// list page, so "published" is never green in one table and grey in another.
pub fn status_class(status: &str) -> &'static str {
    match status {
        "published" => "bg-emerald-100 text-emerald-800",
        "scheduled" => "bg-sky-100 text-sky-800",
        "draft" => "bg-amber-100 text-amber-800",
        "archived" => "bg-ink/10 text-ink/70",
        _ => "bg-mist",
    }
}

/// Escape the few characters that matter in this context. Statuses are already
/// constrained to an allowlist before they reach here; escaping is defence in
/// depth so a future status value can never inject markup.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The human label for a status: the allowlisted word, or the value itself
/// (escaped) for anything unexpected.
fn status_label(status: &str) -> String {
    match status {
        "published" => "Published".to_string(),
        "scheduled" => "Scheduled".to_string(),
        "draft" => "Draft".to_string(),
        "archived" => "Archived".to_string(),
        other => escape(other),
    }
}

/// A status badge, ready to render with `|safe`.
pub fn status_badge(status: &str) -> String {
    format!(
        "<span class=\"rounded px-2 py-0.5 text-xs font-medium {}\">{}</span>",
        status_class(status),
        status_label(status)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_badge_is_consistent_and_escaped() {
        assert!(status_badge("published").contains("Published"));
        assert!(status_badge("published").contains("bg-emerald-100"));
        assert!(status_badge("draft").contains("bg-amber-100"));
        assert!(status_badge("archived").contains("bg-ink/10"));
        // An unknown status still renders, and cannot inject markup.
        let odd = status_badge("<script>evil");
        assert!(odd.contains("&lt;script&gt;"), "got: {odd}");
        assert!(!odd.contains("<script>"));
    }

    #[test]
    fn every_known_status_has_its_own_colour() {
        let classes: std::collections::HashSet<&str> =
            ["published", "scheduled", "draft", "archived"]
                .iter()
                .map(|s| status_class(s))
                .collect();
        assert_eq!(classes.len(), 4, "each status needs a distinct colour");
    }
}
