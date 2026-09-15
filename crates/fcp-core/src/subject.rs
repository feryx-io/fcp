//! Who a piece of data is about, in one canonical form
//! (`docs/COMPLIANCE_FOLLOWUPS.md` §6.1).
//!
//! The same person appears as `sip:1001@pbx.example`, `"Alice"
//! <sips:alice@Pbx.Example;transport=tls>`, `+1 (415) 555-0100` and
//! `tel:+1-415-555-0100`. [`keys`] reduces any of those spellings to the
//! same small set of keys, and two identifiers are about the same subject
//! when their keys meet. The Postgres backend's `fcp_subject_keys` is the
//! same function in SQL; the two are tested against the same spellings.
//!
//! The keys lean towards matching: a number also matches by its last ten
//! digits, so `+14155550100` and `4155550100` meet. That is right for a
//! legal hold, which must not miss what it covers; an erasure narrows what
//! it acts on before it deletes.

/// The canonical keys of an identifier: `num:<digits>` and
/// `tail:<last ten digits>` for a number, `user:<name>` for a name,
/// `aor:<user>@<host>` when there is a host.
pub fn keys(identifier: &str) -> Vec<String> {
    let mut s = identifier.trim();
    // A name-addr's address.
    if let Some(start) = s.find('<') {
        if let Some(len) = s[start + 1..].find('>') {
            s = &s[start + 1..start + 1 + len];
        }
    }
    for scheme in ["sips:", "sip:", "tel:"] {
        if s.len() >= scheme.len() && s[..scheme.len()].eq_ignore_ascii_case(scheme) {
            s = &s[scheme.len()..];
            break;
        }
    }
    let (user, host) = match s.split_once('@') {
        Some((u, h)) => (u, h),
        None => (s, ""),
    };
    let user = user
        .split([';', '?'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let host = host
        .split([';', '?', ':'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();

    let mut out = Vec::new();
    let digits: String = user.chars().filter(|c| c.is_ascii_digit()).collect();
    let numeric = !user.is_empty()
        && !digits.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | '(' | ')' | ' '));
    if numeric {
        out.push(format!("num:{digits}"));
        if digits.len() >= 10 {
            out.push(format!("tail:{}", &digits[digits.len() - 10..]));
        }
    } else if !user.is_empty() {
        out.push(format!("user:{user}"));
    }
    if !user.is_empty() && !host.is_empty() {
        out.push(format!("aor:{user}@{host}"));
    }
    out
}

/// Spellings and the keys they must reduce to: the Rust and SQL
/// implementations are both tested against these.
pub const KEY_VECTORS: &[(&str, &[&str])] = &[
    ("+14155550100", &["num:14155550100", "tail:4155550100"]),
    ("4155550100", &["num:4155550100", "tail:4155550100"]),
    (
        "tel:+1-415-555-0100",
        &["num:14155550100", "tail:4155550100"],
    ),
    ("+1 (415) 555-0100", &["num:14155550100", "tail:4155550100"]),
    (
        "sip:+14155550100@carrier.example;user=phone",
        &[
            "num:14155550100",
            "tail:4155550100",
            "aor:+14155550100@carrier.example",
        ],
    ),
    (
        "sip:1001@pbx.example",
        &["num:1001", "aor:1001@pbx.example"],
    ),
    (
        "\"Alice\" <sips:Alice@Pbx.Example:5061;transport=tls>;tag=abc",
        &["user:alice", "aor:alice@pbx.example"],
    ),
    (
        "alice@example.com",
        &["user:alice", "aor:alice@example.com"],
    ),
    ("alice", &["user:alice"]),
    ("queue:sales", &["user:queue:sales"]),
    ("", &[]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_reduces_to_its_keys() {
        for (spelling, expected) in KEY_VECTORS {
            assert_eq!(keys(spelling), *expected, "{spelling:?}");
        }
    }

    #[test]
    fn a_number_and_its_national_form_meet() {
        let a = keys("+14155550100");
        let b = keys("sip:4155550100@pbx.example");
        assert!(a.iter().any(|k| b.contains(k)));
        let c = keys("sip:+442071234567@x");
        assert!(!a.iter().any(|k| c.contains(k)));
    }
}
