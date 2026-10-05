//! Managed firmware (`docs/PROVISIONING.md` §11,
//! `docs/IDENTITY_AND_DEVICES.md` phase 9).
//!
//! The platform administrator uploads vendors' images, which every tenant
//! sees (Q10): one copy serves everyone. A tenant points a vendor's models
//! at one, for a pilot set of its phones first if it likes; their
//! templates' `firmware_url` then names it, served under the tenant's
//! provisioning URL. What each phone runs comes from its User-Agent, so
//! who is behind is known.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::config_template::{model_matches, PhoneVendor};
use crate::{DeviceId, TenantId};

/// An uploaded image, every tenant's to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareImage {
    pub id: uuid::Uuid,
    pub vendor: PhoneVendor,
    /// The models it is for, as patterns (`T5*`, `VVX 450`); empty is any
    /// of the vendor's.
    pub models: Vec<String>,
    /// The version it installs, as the phones report it in their
    /// User-Agent (`96.86.0.70`).
    pub version: String,
    /// The name it is served under, as the vendor ships it.
    pub file_name: String,
    pub size: u64,
    /// Hex SHA-256 of the file.
    pub sha256: String,
    pub notes: Option<String>,
    pub uploaded_at: SystemTime,
}

impl FirmwareImage {
    /// Whether it is for a phone of `vendor` and `model`.
    pub fn fits(&self, vendor: PhoneVendor, model: Option<&str>) -> bool {
        self.vendor == vendor
            && (self.models.is_empty()
                || model.is_some_and(|m| self.models.iter().any(|p| model_matches(p, m))))
    }
}

/// How far a target has rolled out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RolloutStage {
    /// Only the pilot devices are pointed at the image.
    Pilot,
    /// Every matching phone is.
    All,
}

/// A tenant's choice: these phones run this image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareTarget {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    pub vendor: PhoneVendor,
    /// The models, as a pattern; unset is every model of the vendor.
    pub model_pattern: Option<String>,
    pub image_id: uuid::Uuid,
    pub stage: RolloutStage,
    /// The devices that get it while the stage is `pilot`.
    pub pilot_devices: Vec<DeviceId>,
    pub updated_at: SystemTime,
}

/// A target as the link carries it to a site, with the image it names:
/// images are the platform's, and a site holds those its tenant targets
/// (`docs/PROVISIONING.md` §11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedFirmwareTarget {
    pub target: FirmwareTarget,
    pub image: FirmwareImage,
}

/// The target a phone is pointed at: its vendor's whose pattern names its
/// model most closely (a target with no pattern last), and only while the
/// stage includes it. `None` when none does.
pub fn target_for<'a>(
    targets: &'a [FirmwareTarget],
    vendor: Option<PhoneVendor>,
    model: Option<&str>,
    device: &DeviceId,
) -> Option<&'a FirmwareTarget> {
    let vendor = vendor?;
    let specificity = |p: &str| {
        p.chars()
            .filter(|c| !matches!(c, '*' | '?' | ' ' | '-' | '_'))
            .count()
    };
    targets
        .iter()
        .filter(|t| t.vendor == vendor)
        .filter_map(|t| {
            let rank = match t
                .model_pattern
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
            {
                Some(p) => (1, specificity(p)),
                None => (0, 0),
            };
            let fits = match t
                .model_pattern
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
            {
                Some(p) => model.is_some_and(|m| model_matches(p, m)),
                None => true,
            };
            fits.then_some((rank, t))
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, t)| t)
        .filter(|t| t.stage == RolloutStage::All || t.pilot_devices.contains(device))
}

/// Where a phone stands against its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FirmwareStatus {
    /// It runs the target's version.
    Current,
    /// It runs another.
    Behind,
    /// It has not said what it runs.
    Unknown,
}

impl FirmwareStatus {
    /// `running` against `target`, the versions compared as phones write
    /// them (case and surrounding space aside).
    pub fn of(running: Option<&str>, target: &str) -> Self {
        match running.map(str::trim).filter(|r| !r.is_empty()) {
            None => Self::Unknown,
            Some(r) if r.eq_ignore_ascii_case(target.trim()) => Self::Current,
            Some(_) => Self::Behind,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Behind => "behind",
            Self::Unknown => "unknown",
        }
    }
}

/// An image's file name, as it may be served: a vendor's name — no path,
/// not hidden.
pub fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(pattern: Option<&str>, stage: RolloutStage, pilot: Vec<DeviceId>) -> FirmwareTarget {
        FirmwareTarget {
            id: uuid::Uuid::new_v4(),
            tenant_id: TenantId::new(),
            vendor: PhoneVendor::Yealink,
            model_pattern: pattern.map(String::from),
            image_id: uuid::Uuid::new_v4(),
            stage,
            pilot_devices: pilot,
            updated_at: SystemTime::now(),
        }
    }

    #[test]
    fn a_phone_gets_its_models_target_while_the_stage_includes_it() {
        let (pilot, other) = (DeviceId::new(), DeviceId::new());
        let any = target(None, RolloutStage::All, vec![]);
        let t5 = target(Some("T5*"), RolloutStage::Pilot, vec![pilot]);
        let ts = vec![any.clone(), t5.clone()];
        let yealink = Some(PhoneVendor::Yealink);
        assert_eq!(
            target_for(&ts, yealink, Some("T54W"), &pilot).map(|t| t.id),
            Some(t5.id)
        );
        // Not in the pilot: the T5 target is its, and holds it back.
        assert_eq!(target_for(&ts, yealink, Some("T54W"), &other), None);
        assert_eq!(
            target_for(&ts, yealink, Some("T46U"), &other).map(|t| t.id),
            Some(any.id)
        );
        assert_eq!(
            target_for(&ts, Some(PhoneVendor::Polycom), Some("VVX 450"), &pilot),
            None
        );
        assert_eq!(target_for(&ts, None, Some("T54W"), &pilot), None);
    }

    #[test]
    fn versions_compare_as_phones_write_them() {
        assert_eq!(
            FirmwareStatus::of(Some(" 96.86.0.70"), "96.86.0.70"),
            FirmwareStatus::Current
        );
        assert_eq!(
            FirmwareStatus::of(Some("96.85.0.5"), "96.86.0.70"),
            FirmwareStatus::Behind
        );
        assert_eq!(
            FirmwareStatus::of(None, "96.86.0.70"),
            FirmwareStatus::Unknown
        );
    }

    #[test]
    fn an_image_fits_its_vendors_models_and_names_are_plain() {
        let image = FirmwareImage {
            id: uuid::Uuid::new_v4(),
            vendor: PhoneVendor::Yealink,
            models: vec!["T5*".into()],
            version: "96.86.0.70".into(),
            file_name: "T54W-96.86.0.70.rom".into(),
            size: 1,
            sha256: String::new(),
            notes: None,
            uploaded_at: SystemTime::now(),
        };
        assert!(image.fits(PhoneVendor::Yealink, Some("T57W")));
        assert!(!image.fits(PhoneVendor::Yealink, Some("T46U")));
        assert!(!image.fits(PhoneVendor::Polycom, Some("T57W")));
        assert!(valid_file_name("T54W-96.86.0.70.rom"));
        assert!(!valid_file_name("../etc/passwd"));
        assert!(!valid_file_name("a/b.rom"));
        assert!(!valid_file_name(""));
        assert!(!valid_file_name("..evil"));
    }
}
