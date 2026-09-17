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

pub use fcp_core::trunk::NumberTransform;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_longest_prefix_wins() {
        let t = NumberTransform {
            strip_prefixes: vec!["+".into(), "+1".into()],
            prepend: "1".into(),
            strip_leading: 0,
        };
        assert_eq!(t.apply("+14155550100"), "14155550100");
        assert_eq!(t.apply("+443300030000"), "1443300030000");
    }

    #[test]
    fn test_zero_zero_to_plus() {
        let t = NumberTransform {
            strip_prefixes: vec!["00".into()],
            prepend: "+".into(),
            strip_leading: 0,
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
            strip_leading: 0,
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
            strip_leading: 0,
        };
        assert!(bad.validate().is_err());

        let bad = NumberTransform {
            strip_prefixes: vec![],
            prepend: "x".into(),
            strip_leading: 0,
        };
        assert!(bad.validate().is_err());
    }
}
