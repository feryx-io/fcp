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

/// An identifier's parts, as [`keys`] reads them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Parts {
    /// The digits of a numeric user part.
    digits: Option<String>,
    /// A non-numeric user part, lower-cased.
    name: Option<String>,
    /// `user@host`, when there is a host.
    aor: Option<String>,
}

fn parts(identifier: &str) -> Parts {
    let mut p = Parts::default();
    for key in keys(identifier) {
        if let Some(d) = key.strip_prefix("num:") {
            p.digits = Some(d.to_string());
        } else if let Some(n) = key.strip_prefix("user:") {
            p.name = Some(n.to_string());
        } else if let Some(a) = key.strip_prefix("aor:") {
            p.aor = Some(a.to_string());
        }
    }
    p
}

/// Whether two digit strings are the same number: equal, or one the other
/// with a country code before it — the shorter at least ten digits, so a
/// seven-digit local number or an extension never meets a longer one.
fn same_number(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short == long || (short.len() >= 10 && long.ends_with(short))
}

/// Decides whether an identifier found in data is the subject's, narrowly
/// enough to act on: what an export hands over or an erasure removes
/// (`docs/COMPLIANCE_FOLLOWUPS.md` §6.1). Where [`keys`] lean towards
/// matching, this does not:
///
/// - a number matches the same digits, or its national form (at least ten
///   digits) against its international one — not a different country's
///   number that ends alike;
/// - an address with a host matches only the same user at the same host,
///   so the tenant's `alice` is not `alice@elsewhere.example`;
/// - a bare name matches only the same bare name.
#[derive(Debug, Clone, Default)]
pub struct SubjectMatcher {
    numbers: Vec<String>,
    names: Vec<String>,
    aors: Vec<String>,
}

impl SubjectMatcher {
    /// A matcher for a subject known by `identifiers` (an expansion).
    pub fn new<I, S>(identifiers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut m = Self::default();
        for identifier in identifiers {
            let p = parts(identifier.as_ref());
            if let Some(d) = p.digits {
                push_unique(&mut m.numbers, d);
            }
            if let Some(n) = p.name {
                push_unique(&mut m.names, n);
            }
            if let Some(a) = p.aor {
                push_unique(&mut m.aors, a);
            }
        }
        m
    }

    /// Whether the matcher knows no identifier at all.
    pub fn is_empty(&self) -> bool {
        self.numbers.is_empty() && self.names.is_empty() && self.aors.is_empty()
    }

    /// Whether `identifier` — a URI, a name-addr, a number, a name — is the
    /// subject's.
    pub fn matches(&self, identifier: &str) -> bool {
        let p = parts(identifier);
        if let Some(d) = &p.digits {
            return self.numbers.iter().any(|n| same_number(n, d));
        }
        match (&p.aor, &p.name) {
            (Some(aor), _) => self.aors.contains(aor),
            (None, Some(name)) => self.names.contains(name),
            (None, None) => false,
        }
    }

    /// Whether any of `identifiers` is the subject's.
    pub fn matches_any<'a>(&self, identifiers: impl IntoIterator<Item = &'a str>) -> bool {
        identifiers.into_iter().any(|i| self.matches(i))
    }
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !v.contains(&s) {
        v.push(s);
    }
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
    fn the_matcher_takes_the_subject_and_nobody_who_looks_like_them() {
        let m = SubjectMatcher::new([
            "alice",
            "1001",
            "alice@pbx.example",
            "1001@pbx.example",
            "+14155550100",
            "alice@mail.example",
        ]);
        for theirs in [
            "sip:alice@pbx.example",
            "\"Alice\" <sips:Alice@PBX.example:5061;transport=tls>",
            "sip:1001@pbx.example",
            "sip:1001@10.0.0.5",
            "tel:+1-415-555-0100",
            "sip:4155550100@carrier.example",
            "alice",
            "alice@mail.example",
        ] {
            assert!(m.matches(theirs), "{theirs:?} is the subject's");
        }
        for not_theirs in [
            "sip:alice@elsewhere.example",
            "sip:bob@pbx.example",
            "sip:10011@pbx.example",
            "sip:+444155550100@carrier.example",
            "5550100",
            "bob",
            "",
        ] {
            assert!(!m.matches(not_theirs), "{not_theirs:?} is not");
        }
        assert!(m.matches_any(["sip:bob@pbx.example", "sip:alice@pbx.example"]));
        assert!(SubjectMatcher::new(Vec::<String>::new()).is_empty());
        assert!(!SubjectMatcher::new(["", "   "]).matches("alice"));
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
