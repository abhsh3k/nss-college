//! Calendar-date validation.
//!
//! The project deliberately keeps a date-parsing crate out of the build (the
//! `time` crate's parser is behind a feature that is not enabled), so the one
//! rule every handler needs lives here instead of being reimplemented per
//! module. Before this, three copies existed (`routes::admin::is_valid_date`,
//! `routes::admin::exams::valid_date`, `services::attendance::valid_date`) and
//! they disagreed: only this one rejected a year outside 2000–2100 and only
//! `routes::admin` trimmed the input. A single implementation means a date is
//! accepted or rejected the same way on every form.

/// True when `s` is a real calendar date written as `YYYY-MM-DD`.
///
/// The year is bounded to 2000–2100: this is a college system, and a typo like
/// `1902-05-04` should be a form error rather than a plausible-looking record.
pub fn valid_date(s: &str) -> bool {
    let mut parts = s.trim().split('-');
    let (Some(y), Some(m), Some(d), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return false;
    }
    if !y.bytes().chain(m.bytes()).chain(d.bytes()).all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let max = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (2000..=2100).contains(&y) && d >= 1 && d <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_real_date() {
        assert!(valid_date("2026-10-11"));
        assert!(valid_date("2024-02-29"), "leap day in a leap year");
        assert!(valid_date("2000-01-01"));
    }

    #[test]
    fn rejects_impossible_days() {
        assert!(!valid_date("2023-02-29"), "2023 is not a leap year");
        assert!(!valid_date("2024-04-31"), "April has 30 days");
        assert!(!valid_date("2024-13-01"), "there is no month 13");
        assert!(!valid_date("2024-00-10"), "there is no month 0");
        assert!(!valid_date("2024-05-00"), "there is no day 0");
    }

    #[test]
    fn rejects_the_wrong_shape() {
        assert!(!valid_date("2024-1-01"), "month must be zero-padded");
        assert!(!valid_date("24-01-01"), "year must be four digits");
        assert!(!valid_date("2024/01/01"));
        assert!(!valid_date("2024-01-01T00:00"));
        assert!(!valid_date(""));
        assert!(!valid_date("2024-01-01-01"));
    }

    #[test]
    fn rejects_a_silly_year() {
        assert!(!valid_date("1999-12-31"));
        assert!(!valid_date("2101-01-01"));
    }

    #[test]
    fn ignores_surrounding_whitespace() {
        assert!(valid_date("  2026-10-11  "));
    }
}
