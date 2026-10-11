//! Shared, server-side input limits.
//!
//! Every form field is bounded by a column in the schema, but Postgres `TEXT`
//! has no length, so an authenticated user (or an attacker with a session) can
//! post a megabyte of text into a title and the row is happily written. These
//! limits are the application-side ceiling: generous enough for a real title or
//! name, small enough that a single field cannot be used to store junk or to
//! make every rendered page enormous.
//!
//! The rule is applied at the point the value is read from the request, so a
//! rejected value becomes a form error and never reaches the database.

/// A person's name, a login identifier, a category label.
pub const MAX_NAME: usize = 120;
/// A title: a notice, a news item, an event, a document, a course title.
pub const MAX_TITLE: usize = 200;
/// A course code or a short key (an admission number, a PRN).
pub const MAX_CODE: usize = 40;
/// A free-text reason, note or comment attached to an audited action.
pub const MAX_NOTE: usize = 1_000;
/// The body of an article, a notice or a page.
pub const MAX_BODY: usize = 100_000;
/// An email address. The RFC's practical maximum (especially with a signature).
pub const MAX_EMAIL: usize = 254;
/// A password. Bounded so a huge body cannot be used to burn CPU in Argon2.
pub const MAX_PASSWORD: usize = 200;

/// True when `s` is no longer than `max` characters.
///
/// Counted in `char`s, not bytes: the limit is about how much a person typed,
/// and counting bytes would reject a short Devanagari or accented value early.
pub fn fits(s: &str, max: usize) -> bool {
    s.chars().count() <= max
}

/// True when `s` is within `max` characters once trimmed, and is not empty.
pub fn present_and_fits(s: &str, max: usize) -> bool {
    let s = s.trim();
    !s.is_empty() && fits(s, max)
}

/// The label to show a person when `s` is too long for `max` characters.
pub fn too_long_message(label: &str, max: usize) -> String {
    format!("{label} must be {max} characters or fewer.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_characters_not_bytes() {
        // Ten three-byte characters are within a limit of ten even though they
        // are thirty bytes.
        let devanagari = "अआइईउऊऋएऐओ";
        assert_eq!(devanagari.chars().count(), 10);
        assert_eq!(devanagari.len(), 30);
        assert!(fits(devanagari, 10));
        assert!(!fits(devanagari, 9));
    }

    #[test]
    fn an_empty_required_value_does_not_pass() {
        assert!(!present_and_fits("   ", 10));
        assert!(present_and_fits("  ok  ", 10));
    }

    #[test]
    fn the_boundary_is_inclusive() {
        assert!(fits(&"x".repeat(MAX_TITLE), MAX_TITLE));
        assert!(!fits(&"x".repeat(MAX_TITLE + 1), MAX_TITLE));
    }
}
