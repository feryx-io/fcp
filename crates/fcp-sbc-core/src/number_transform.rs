//! Number manipulation for the SBC transform stage.
//!
//! Per-trunk digit transforms applied to the dialed number when an egress
//! route is built — after routing has chosen the trunk, so each carrier can
//! receive numbers in the format it expects (e.g. strip `+`/`00` and prepend
//! a national prefix, or normalize to E.164).
//!
//! This is deliberately a digit-plan primitive, distinct from header
//! manipulation (HMR): HMR rewrites SIP headers by regex per message, while a
//! `NumberTransform` rewrites the dialed number as part of the routing
//! decision.

use serde::{Deserialize, Serialize};

/// A dialed-number transform attached to a trunk (egress direction).
///
/// Applied to the user part of the Request-URI when building the outbound
/// route: first the longest matching entry of `strip_prefixes` is removed,
/// then `prepend` is added.
///
/// Examples:
/// - Send E.164 without `+` to a carrier: `strip_prefixes = ["+"]`
/// - US carrier expecting 11 digits: `strip_prefixes = ["+1", "+"]`, `prepend = "1"`
///   (`+14155550100` → `14155550100`; `+44330030000` → `1443300030000` is
///   avoided because `+1` is checked first — order entries longest-first)
/// - Convert `00`-dialing to `+`: `strip_prefixes = ["00"]`, `prepend = "+"`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NumberTransform {
    /// Prefixes to strip; the longest matching entry wins. Optional.
    #[serde(default)]
    pub strip_prefixes: Vec<String>,

    /// String prepended after stripping. Optional.
    #[serde(default)]
    pub prepend: String,
}

impl NumberTransform {
    /// Apply the transform to a dialed number (the URI user part).
    ///
    /// Non-numeric user parts (named users like `alice`) are returned
    /// unchanged: number transforms only apply to strings that consist of
    /// digits after an optional leading `+` and separators.
    pub fn apply(&self, number: &str) -> String {
        if !Self::looks_like_number(number) {
            return number.to_string();
        }

        let stripped = self
            .strip_prefixes
            .iter()
            .filter(|p| !p.is_empty() && number.starts_with(p.as_str()))
            .max_by_key(|p| p.len())
            .map(|p| &number[p.len()..])
            .unwrap_or(number);

        format!("{}{}", self.prepend, stripped)
    }

    /// Whether the transform changes anything.
    pub fn is_noop(&self) -> bool {
        self.strip_prefixes.is_empty() && self.prepend.is_empty()
    }

    /// Validate configuration values.
    pub fn validate(&self) -> Result<(), String> {
        for prefix in &self.strip_prefixes {
            if prefix.is_empty() {
                return Err("number_transform.strip_prefixes entries must not be empty".into());
            }
            if !prefix.chars().all(|c| c.is_ascii_digit() || c == '+') {
                return Err(format!(
                    "number_transform.strip_prefixes entry '{prefix}' may only contain digits and '+'"
                ));
            }
        }
        if !self
            .prepend
            .chars()
            .all(|c| c.is_ascii_digit() || c == '+' || c == '*' || c == '#')
        {
            return Err(format!(
                "number_transform.prepend '{}' may only contain digits, '+', '*', '#'",
                self.prepend
            ));
        }
        Ok(())
    }

    fn looks_like_number(user: &str) -> bool {
        let significant: Vec<char> = user
            .chars()
            .filter(|c| !matches!(c, '-' | '.' | '(' | ')' | ' '))
            .collect();
        match significant.split_first() {
            Some(('+', rest)) => !rest.is_empty() && rest.iter().all(|c| c.is_ascii_digit()),
            Some(_) => significant.iter().all(|c| c.is_ascii_digit()),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_longest_prefix_wins() {
        let t = NumberTransform {
            strip_prefixes: vec!["+".into(), "+1".into()],
            prepend: "1".into(),
        };
        assert_eq!(t.apply("+14155550100"), "14155550100");
        assert_eq!(t.apply("+443300030000"), "1443300030000");
    }

    #[test]
    fn test_zero_zero_to_plus() {
        let t = NumberTransform {
            strip_prefixes: vec!["00".into()],
            prepend: "+".into(),
        };
        assert_eq!(t.apply("00443300030000"), "+443300030000");
        // No prefix match: prepend still applies
        assert_eq!(t.apply("14155550100"), "+14155550100");
    }

    #[test]
    fn test_named_users_pass_through() {
        let t = NumberTransform {
            strip_prefixes: vec!["+".into()],
            prepend: "9".into(),
        };
        assert_eq!(t.apply("alice"), "alice");
        assert_eq!(t.apply("bob2"), "bob2");
        assert_eq!(t.apply(""), "");
    }

    #[test]
    fn test_noop_and_validate() {
        assert!(NumberTransform::default().is_noop());
        assert!(NumberTransform::default().validate().is_ok());

        let bad = NumberTransform {
            strip_prefixes: vec!["abc".into()],
            prepend: String::new(),
        };
        assert!(bad.validate().is_err());

        let bad = NumberTransform {
            strip_prefixes: vec![],
            prepend: "x".into(),
        };
        assert!(bad.validate().is_err());
    }
}
