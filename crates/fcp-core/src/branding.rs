//! A tenant's brand (`docs/BRANDING.md`, `docs/SOFTPHONE.md` §2.5): the
//! name, colors, logos, ringtones, links and dial-plan hints the softphone
//! takes on when one of the tenant's people signs in. The contract is
//! Anvil's `docs/BRANDING.md` v1, served at `/brand/v1`.
//!
//! Assets (logos, the icon, ringtones) are uploaded first and named by
//! their SHA-256; the brand refers to them by hash, and they are served
//! publicly by hash. They are small (at most [`MAX_ASSET_BYTES`]), so they
//! are kept in the database with the brand, which every admin and every
//! backup sees.

use std::collections::BTreeMap;
use std::time::SystemTime;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::TenantId;

/// The contract's version, `schema_version` in what `/brand/v1` serves.
pub const SCHEMA_VERSION: u32 = 1;

/// The largest asset taken: the contract's limit on what an app fetches.
pub const MAX_ASSET_BYTES: usize = 5 * 1024 * 1024;

/// The most ringtones a brand offers.
pub const MAX_RINGTONES: usize = 8;

/// The colors of one theme, each `#RRGGBB` or `#RRGGBBAA`; one unset is
/// the app's own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrandColors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BrandColors {
    fn all(&self) -> [(&'static str, &Option<String>); 7] {
        [
            ("primary", &self.primary),
            ("accent", &self.accent),
            ("background", &self.background),
            ("surface", &self.surface),
            ("on_primary", &self.on_primary),
            ("on_surface", &self.on_surface),
            ("error", &self.error),
        ]
    }

    /// The first color that is not one, by its slot.
    pub fn invalid(&self) -> Option<&'static str> {
        self.all()
            .into_iter()
            .find(|(_, c)| c.as_deref().is_some_and(|c| !valid_color(c)))
            .map(|(slot, _)| slot)
    }
}

/// The tenant's links, each `https://`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrandLinks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
}

impl BrandLinks {
    /// The first link that is not an `https://` URL, by its name.
    pub fn invalid(&self) -> Option<&'static str> {
        [
            ("support", &self.support),
            ("privacy", &self.privacy),
            ("terms", &self.terms),
            ("website", &self.website),
        ]
        .into_iter()
        .find(|(_, l)| l.as_deref().is_some_and(|l| !valid_link(l)))
        .map(|(name, _)| name)
    }
}

/// Hints for the app's dialer: prefixes it takes off what is dialed, and
/// short codes it turns into addresses.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrandDialPlan {
    #[serde(default)]
    pub prefix_strip: Vec<String>,
    #[serde(default)]
    pub short_codes: BTreeMap<String, String>,
}

/// A ringtone the app offers, by its asset's hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BrandRingtone {
    pub name: String,
    #[serde(default)]
    pub default: bool,
    pub sha256: String,
}

/// A tenant's brand. Assets are named by their SHA-256 and must be the
/// tenant's uploads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantBrand {
    pub tenant_id: TenantId,
    pub app_name: String,
    #[serde(default)]
    pub colors: BrandColors,
    #[serde(default)]
    pub dark_colors: Option<BrandColors>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub logo_dark: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub ringtones: Vec<BrandRingtone>,
    #[serde(default)]
    pub links: BrandLinks,
    #[serde(default)]
    pub dial_plan: BrandDialPlan,
    pub updated_at: SystemTime,
}

impl TenantBrand {
    /// Every asset the brand names, each once, in the order it names them.
    pub fn asset_hashes(&self) -> Vec<&str> {
        let mut hashes: Vec<&str> = Vec::new();
        let named = [&self.logo, &self.logo_dark, &self.icon]
            .into_iter()
            .filter_map(|h| h.as_deref())
            .chain(self.ringtones.iter().map(|r| r.sha256.as_str()));
        for h in named {
            if !hashes.contains(&h) {
                hashes.push(h);
            }
        }
        hashes
    }
}

/// What kind of file an asset is, from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Image,
    Audio,
}

/// An uploaded file, by its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrandAsset {
    pub tenant_id: TenantId,
    /// Hex SHA-256 of the bytes.
    pub sha256: String,
    /// What the bytes are, as [`sniff_mime`] found.
    pub mime: String,
    pub size: u64,
    /// A PNG's dimensions.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The name it was uploaded under, for the admin's page.
    pub file_name: Option<String>,
    pub uploaded_at: SystemTime,
}

impl BrandAsset {
    pub fn kind(&self) -> Option<AssetKind> {
        kind_of(&self.mime)
    }
}

/// A brand as the link carries it to a site, with what its assets are
/// (`docs/HYBRID_CONTROL_PLANE.md`); the link fetches the bytes it lacks
/// by hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedBrand {
    pub brand: TenantBrand,
    pub assets: Vec<BrandAsset>,
}

/// Where brands and their assets are kept.
#[async_trait]
pub trait BrandStore: Send + Sync {
    async fn get_brand(&self, tenant_id: &TenantId) -> anyhow::Result<Option<TenantBrand>>;
    async fn put_brand(&self, brand: &TenantBrand) -> anyhow::Result<()>;
    /// Whether there was one.
    async fn delete_brand(&self, tenant_id: &TenantId) -> anyhow::Result<bool>;

    /// Keep an asset; the same bytes uploaded again keep the first.
    async fn put_brand_asset(&self, asset: &BrandAsset, bytes: &[u8]) -> anyhow::Result<()>;
    async fn list_brand_assets(&self, tenant_id: &TenantId) -> anyhow::Result<Vec<BrandAsset>>;
    async fn get_brand_asset(
        &self,
        tenant_id: &TenantId,
        sha256: &str,
    ) -> anyhow::Result<Option<BrandAsset>>;
    /// The asset's bytes, whichever tenant uploaded them: assets are
    /// served by hash, and the same hash is the same file.
    async fn brand_asset_bytes(
        &self,
        sha256: &str,
    ) -> anyhow::Result<Option<(BrandAsset, Vec<u8>)>>;
    /// Whether there was one.
    async fn delete_brand_asset(&self, tenant_id: &TenantId, sha256: &str) -> anyhow::Result<bool>;
}

/// `#RRGGBB` or `#RRGGBBAA`.
pub fn valid_color(color: &str) -> bool {
    color
        .strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// An `https://` URL with a host.
pub fn valid_link(link: &str) -> bool {
    link.len() <= 2048
        && url::Url::parse(link)
            .is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()))
}

/// A hex SHA-256 as assets are named: 64 lowercase hex digits.
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

/// What the bytes are, from their first bytes, among the files a brand
/// takes: PNG, JPEG, WebP or SVG images; Ogg, MP3 or WAV audio. What the
/// uploader said they are is not asked.
pub fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    let riff = |kind: &[u8]| bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == kind;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if riff(b"WEBP") {
        Some("image/webp")
    } else if riff(b"WAVE") {
        Some("audio/wav")
    } else if bytes.starts_with(b"OggS") {
        Some("audio/ogg")
    } else if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        Some("audio/mpeg")
    } else if is_svg(bytes) {
        Some("image/svg+xml")
    } else {
        None
    }
}

fn is_svg(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(&bytes[..bytes.len().min(4096)]) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    (text.starts_with('<')) && text.contains("<svg")
}

/// The kind of file a MIME type names, among those [`sniff_mime`] finds.
pub fn kind_of(mime: &str) -> Option<AssetKind> {
    match mime {
        "image/png" | "image/jpeg" | "image/webp" | "image/svg+xml" => Some(AssetKind::Image),
        "audio/ogg" | "audio/mpeg" | "audio/wav" => Some(AssetKind::Audio),
        _ => None,
    }
}

/// A PNG's width and height, from its header.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_hex_with_or_without_alpha() {
        assert!(valid_color("#1A73E8"));
        assert!(valid_color("#1a73e8cc"));
        assert!(!valid_color("1A73E8"));
        assert!(!valid_color("#1A73E"));
        assert!(!valid_color("#GGGGGG"));
        assert!(!valid_color("red"));
        let colors = BrandColors {
            primary: Some("#000000".into()),
            on_surface: Some("blue".into()),
            ..Default::default()
        };
        assert_eq!(colors.invalid(), Some("on_surface"));
    }

    #[test]
    fn links_are_https() {
        assert!(valid_link("https://acme.example/support"));
        assert!(!valid_link("http://acme.example"));
        assert!(!valid_link("javascript:alert(1)"));
        assert!(!valid_link("https://"));
        let links = BrandLinks {
            terms: Some("ftp://acme.example".into()),
            ..Default::default()
        };
        assert_eq!(links.invalid(), Some("terms"));
    }

    #[test]
    fn files_are_known_by_their_bytes() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&512u32.to_be_bytes());
        png.extend_from_slice(&256u32.to_be_bytes());
        assert_eq!(sniff_mime(&png), Some("image/png"));
        assert_eq!(png_dimensions(&png), Some((512, 256)));
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_mime(b"RIFF\0\0\0\0WAVEfmt "), Some("audio/wav"));
        assert_eq!(sniff_mime(b"OggS\0\x02"), Some("audio/ogg"));
        assert_eq!(sniff_mime(b"ID3\x04\0"), Some("audio/mpeg"));
        assert_eq!(
            sniff_mime(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            Some("image/svg+xml")
        );
        assert_eq!(sniff_mime(b"<html><body>no</body></html>"), None);
        assert_eq!(sniff_mime(b"MZ\x90\0"), None);
        assert_eq!(kind_of("image/svg+xml"), Some(AssetKind::Image));
        assert_eq!(kind_of("audio/wav"), Some(AssetKind::Audio));
        assert_eq!(kind_of("text/html"), None);
    }

    #[test]
    fn a_brand_names_each_asset_once() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let brand = TenantBrand {
            tenant_id: TenantId::new(),
            app_name: "Acme Voice".into(),
            colors: BrandColors::default(),
            dark_colors: None,
            logo: Some(a.clone()),
            logo_dark: Some(a.clone()),
            icon: None,
            ringtones: vec![BrandRingtone {
                name: "Ring".into(),
                default: true,
                sha256: b.clone(),
            }],
            links: BrandLinks::default(),
            dial_plan: BrandDialPlan::default(),
            updated_at: SystemTime::now(),
        };
        assert_eq!(brand.asset_hashes(), vec![a.as_str(), b.as_str()]);
        assert!(valid_hash(&a));
        assert!(!valid_hash("A".repeat(64).as_str()));
        assert!(!valid_hash("../x"));
    }
}
