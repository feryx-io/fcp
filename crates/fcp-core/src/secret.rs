//! Comparing a secret to what someone presented.
//!
//! An API token, a provisioning token, a session token: every one of them
//! is checked by comparing what arrived against what the node holds, and
//! the ordinary `==` on a `str` stops at the first byte that differs. How
//! long that takes is a measurement of how much of the secret the caller
//! guessed right, so a caller who can time the answer can learn it one
//! byte at a time rather than guessing the whole thing.
//!
//! There is one implementation of the comparison here because there were
//! ten of it — identical, spread across nine binaries — and three more
//! places that had none and used `==` (`docs/EDGE_SECURITY.md` §2.6).

/// Whether two secrets are the same, in time that does not depend on how
/// much of them matches.
///
/// The **length** is not hidden: a mismatch in length answers immediately.
/// That is the usual trade, and it is the right one here — the tokens this
/// compares are a fixed width the node chose, so their length is not a
/// secret, and hiding it would mean hashing on every request.
pub fn constant_time_eq(presented: impl AsRef<[u8]>, expected: impl AsRef<[u8]>) -> bool {
    let (presented, expected) = (presented.as_ref(), expected.as_ref());
    if presented.len() != expected.len() {
        return false;
    }
    presented
        .iter()
        .zip(expected)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_secret_matches() {
        assert!(constant_time_eq("fcp_abc123", "fcp_abc123"));
        assert!(constant_time_eq("", ""));
        assert!(constant_time_eq(b"bytes".as_slice(), b"bytes".as_slice()));
    }

    #[test]
    fn a_different_secret_does_not() {
        assert!(!constant_time_eq("fcp_abc123", "fcp_abc124"));
        // A difference in the first byte is as false as one in the last,
        // which is the whole point.
        assert!(!constant_time_eq("Xcp_abc123", "fcp_abc123"));
        assert!(!constant_time_eq("fcp_abc123", "fcp_abc12X"));
    }

    #[test]
    fn a_prefix_is_not_the_secret() {
        assert!(!constant_time_eq("fcp_abc", "fcp_abc123"));
        assert!(!constant_time_eq("fcp_abc123", "fcp_abc"));
        assert!(!constant_time_eq("", "fcp_abc123"));
    }

    #[test]
    fn it_compares_bytes_not_characters() {
        // Two strings that differ only in case are different secrets.
        assert!(!constant_time_eq("Token", "token"));
    }
}
