//! The location object an emergency call carries by value
//! (`docs/E911_BLOCK_2.md` §5.2).
//!
//! A PIDF-LO (RFC 4119) as RFC 5491 profiles it: one `presence` whose entity
//! is the caller, one `dm:device` (RFC 4479) holding a `gp:geopriv` with the
//! civic address (RFC 5139), and, when the site has coordinates, a second
//! `gp:geopriv` with a `gml:Point` — each location in its own `geopriv`,
//! so a recipient that understands only one form still finds it whole.
//! Usage rules allow retransmission: the location is for routing and
//! dispatch.

use crate::e911::{DispatchableLocation, GeoPoint};
use crate::phone_number::E911Address;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;
use std::io::Cursor;
use std::time::SystemTime;

pub const PIDF_NS: &str = "urn:ietf:params:xml:ns:pidf";
pub const DATA_MODEL_NS: &str = "urn:ietf:params:xml:ns:pidf:data-model";
pub const GEOPRIV_NS: &str = "urn:ietf:params:xml:ns:pidf:geopriv10";
pub const BASIC_POLICY_NS: &str = "urn:ietf:params:xml:ns:pidf:geopriv10:basicPolicy";
pub const CIVIC_NS: &str = "urn:ietf:params:xml:ns:pidf:geopriv10:civicAddr";
pub const GML_NS: &str = "http://www.opengis.net/gml";
/// The content type of a PIDF-LO body part.
pub const CONTENT_TYPE: &str = "application/pidf+xml";

/// Who the location object is about and when it was made.
#[derive(Debug, Clone)]
pub struct Subject<'a> {
    /// The presentity: `pres:` and the caller's address
    /// (`pres:+15185550100@pbx.example`).
    pub entity: &'a str,
    /// The device's identifier, a URI (`urn:uuid:…`).
    pub device_id: &'a str,
    pub timestamp: SystemTime,
}

/// The PIDF-LO document for `location`.
///
/// An address with no structured street (no street name and house number)
/// is written with its street line as `RD`: sent with what can be said
/// about it rather than withheld (§1, decision 3).
pub fn document(location: &DispatchableLocation, subject: &Subject<'_>) -> String {
    let mut w = Writer::new(Cursor::new(Vec::new()));
    let ok = "PIDF-LO writes to memory";
    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .expect(ok);

    let mut presence = BytesStart::new("presence");
    presence.push_attribute(("xmlns", PIDF_NS));
    presence.push_attribute(("xmlns:dm", DATA_MODEL_NS));
    presence.push_attribute(("xmlns:gp", GEOPRIV_NS));
    presence.push_attribute(("xmlns:gbp", BASIC_POLICY_NS));
    presence.push_attribute(("xmlns:ca", CIVIC_NS));
    if location.point.is_some() {
        presence.push_attribute(("xmlns:gml", GML_NS));
    }
    presence.push_attribute(("entity", subject.entity));
    w.write_event(Event::Start(presence)).expect(ok);

    let mut device = BytesStart::new("dm:device");
    device.push_attribute(("id", "emergency-caller"));
    w.write_event(Event::Start(device)).expect(ok);

    let method = location.source.pidf_method();
    geopriv(&mut w, method, |w| civic_address(w, location));
    if let Some(point) = location.point {
        geopriv(&mut w, method, |w| gml_point(w, point));
    }

    text_element(&mut w, "dm:deviceID", subject.device_id);
    let timestamp = chrono::DateTime::<chrono::Utc>::from(subject.timestamp)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    text_element(&mut w, "dm:timestamp", &timestamp);

    w.write_event(Event::End(BytesEnd::new("dm:device")))
        .expect(ok);
    w.write_event(Event::End(BytesEnd::new("presence")))
        .expect(ok);
    String::from_utf8(w.into_inner().into_inner()).expect("PIDF-LO is UTF-8")
}

fn geopriv(
    w: &mut Writer<Cursor<Vec<u8>>>,
    method: &str,
    location_info: impl FnOnce(&mut Writer<Cursor<Vec<u8>>>),
) {
    let ok = "PIDF-LO writes to memory";
    start(w, "gp:geopriv");
    start(w, "gp:location-info");
    location_info(w);
    end(w, "gp:location-info");
    start(w, "gp:usage-rules");
    text_element(w, "gbp:retransmission-allowed", "true");
    end(w, "gp:usage-rules");
    text_element(w, "gp:method", method);
    w.write_event(Event::End(BytesEnd::new("gp:geopriv")))
        .expect(ok);
}

/// The RFC 5139 elements, in the schema's sequence order.
fn civic_address(w: &mut Writer<Cursor<Vec<u8>>>, location: &DispatchableLocation) {
    let a = &location.address;
    let c = &a.civic;
    let structured = c.is_structured();
    let mut element = BytesStart::new("ca:civicAddress");
    element.push_attribute(("xml:lang", "en"));
    w.write_event(Event::Start(element))
        .expect("PIDF-LO writes to memory");

    let address2 = a.address2.as_deref().and_then(line);
    // A second address line not already carried as floor, building, unit
    // or room is additional location information.
    let loc = opt(&c.additional).or_else(|| {
        let in_elements = [&c.floor, &c.building, &c.unit, &c.room]
            .iter()
            .any(|v| opt(v).is_some());
        if in_elements {
            None
        } else {
            address2
        }
    });
    let road = if structured {
        opt(&c.street_name)
    } else {
        line(&a.address1)
    };

    let elements: [(&str, Option<&str>); 16] = [
        ("ca:country", line(&a.country)),
        ("ca:A1", line(&a.state)),
        ("ca:A3", line(&a.city)),
        ("ca:PRD", opt(&c.street_predirection).filter(|_| structured)),
        ("ca:RD", road),
        ("ca:STS", opt(&c.street_suffix).filter(|_| structured)),
        (
            "ca:POD",
            opt(&c.street_postdirection).filter(|_| structured),
        ),
        ("ca:HNO", opt(&c.house_number).filter(|_| structured)),
        ("ca:HNS", opt(&c.house_number_suffix).filter(|_| structured)),
        ("ca:LOC", loc),
        ("ca:FLR", opt(&c.floor)),
        (
            "ca:NAM",
            opt(&c.name).or(location.location_name.as_deref().and_then(line)),
        ),
        ("ca:PC", line(&a.postal_code)),
        ("ca:BLD", opt(&c.building)),
        ("ca:UNIT", opt(&c.unit)),
        ("ca:ROOM", opt(&c.room)),
    ];
    for (name, value) in elements {
        if let Some(value) = value {
            text_element(w, name, value);
        }
    }
    end(w, "ca:civicAddress");
}

/// A set element's trimmed text, when there is any.
fn opt(v: &Option<String>) -> Option<&str> {
    v.as_deref().and_then(line)
}

fn line(v: &str) -> Option<&str> {
    Some(v.trim()).filter(|s| !s.is_empty())
}

fn gml_point(w: &mut Writer<Cursor<Vec<u8>>>, point: GeoPoint) {
    let mut element = BytesStart::new("gml:Point");
    element.push_attribute(("srsName", "urn:ogc:def:crs:EPSG::4326"));
    w.write_event(Event::Start(element))
        .expect("PIDF-LO writes to memory");
    // EPSG::4326 axis order: latitude, then longitude.
    text_element(
        w,
        "gml:pos",
        &format!("{} {}", point.latitude, point.longitude),
    );
    end(w, "gml:Point");
}

fn start(w: &mut Writer<Cursor<Vec<u8>>>, name: &str) {
    w.write_event(Event::Start(BytesStart::new(name)))
        .expect("PIDF-LO writes to memory");
}

fn end(w: &mut Writer<Cursor<Vec<u8>>>, name: &str) {
    w.write_event(Event::End(BytesEnd::new(name)))
        .expect("PIDF-LO writes to memory");
}

fn text_element(w: &mut Writer<Cursor<Vec<u8>>>, name: &str, text: &str) {
    start(w, name);
    // XML 1.0 has no place for control characters; an address that holds
    // one is sent without it rather than as a document nobody can parse.
    let text: String = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect();
    w.write_event(Event::Text(BytesText::new(&text)))
        .expect("PIDF-LO writes to memory");
    end(w, name);
}

/// A location a device sent in a PIDF-LO (`docs/E911_BLOCK_2.md` §6.3).
#[derive(Debug, Clone, PartialEq)]
pub struct SuppliedLocation {
    /// The civic address, when the document carried one.
    pub address: Option<E911Address>,
    /// The geodetic point, when it carried one.
    pub point: Option<GeoPoint>,
    /// The `gp:method` the document claimed (`GPS`, `Wiremap`, `Manual`, …).
    pub method: Option<String>,
}

impl SuppliedLocation {
    /// Whether anything usable was found.
    pub fn is_empty(&self) -> bool {
        self.address.is_none() && self.point.is_none()
    }
}

/// Read a PIDF-LO a device sent: the first civic address and the first
/// geodetic point in it, whatever prefixes the sender used and wherever in
/// the document they sit (a `tuple`, a `device` or a `person`, RFC 5491
/// §3). `None` when the XML does not parse; an empty result when it holds
/// no location this platform understands.
///
/// The address is a caller's claim: validate it (and the caller's right to
/// make it) before dispatching on it.
pub fn parse(xml: &str) -> Option<SuppliedLocation> {
    use quick_xml::name::ResolveResult;
    use quick_xml::NsReader;

    let mut reader = NsReader::from_str(xml);
    // A document that ends mid-element, or closes what it never opened, is
    // not a location object.
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    let mut civic: Option<E911Address> = None;
    let mut civic_elements: Vec<(String, String)> = Vec::new();
    let mut in_civic = false;
    let mut point: Option<GeoPoint> = None;
    let mut method: Option<String> = None;
    // The element whose text is being collected, and the text so far.
    let mut collecting: Option<String> = None;
    let mut text = String::new();
    let mut pos: Option<String> = None;
    // Open elements, so a document that ends inside one is refused.
    let mut depth: i32 = 0;

    loop {
        let (namespace, event) = reader.read_resolved_event().ok()?;
        let namespace = match namespace {
            ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).to_string(),
            _ => String::new(),
        };
        match event {
            Event::Start(e) => {
                depth += 1;
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                match (namespace.as_str(), local.as_str()) {
                    (CIVIC_NS, "civicAddress") if civic.is_none() => {
                        in_civic = true;
                        civic_elements.clear();
                    }
                    (GML_NS, "Point") => pos = Some(String::new()),
                    _ => {}
                }
                collecting = Some(local);
                text.clear();
            }
            Event::Text(t) => {
                text.push_str(&t.decode().ok()?);
            }
            Event::GeneralRef(r) => {
                text.push_str(match r.decode().ok()?.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
            }
            Event::End(e) => {
                depth -= 1;
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                let value = text.trim().to_string();
                text.clear();
                if collecting.as_deref() == Some(local.as_str()) {
                    collecting = None;
                }
                match (namespace.as_str(), local.as_str()) {
                    (CIVIC_NS, "civicAddress") => {
                        in_civic = false;
                        civic = Some(address_from_elements(&civic_elements));
                    }
                    (CIVIC_NS, _) if in_civic && !value.is_empty() => {
                        civic_elements.push((local, value));
                    }
                    (GML_NS, "pos") if pos.is_some() => pos = Some(value),
                    (GML_NS, "Point") => {
                        point = point.or_else(|| pos.as_deref().and_then(parse_pos));
                    }
                    (GEOPRIV_NS, "method") if method.is_none() && !value.is_empty() => {
                        method = Some(value);
                    }
                    _ => {}
                }
            }
            Event::Eof if depth == 0 => break,
            Event::Eof => return None,
            _ => {}
        }
    }
    Some(SuppliedLocation {
        address: civic,
        point,
        method,
    })
}

/// `latitude longitude` in degrees (EPSG:4326 axis order).
fn parse_pos(pos: &str) -> Option<GeoPoint> {
    let mut parts = pos.split_whitespace();
    let latitude: f64 = parts.next()?.parse().ok()?;
    let longitude: f64 = parts.next()?.parse().ok()?;
    ((-90.0..=90.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude)).then_some(
        GeoPoint {
            latitude,
            longitude,
        },
    )
}

/// The RFC 5139 elements as an address: the elements this platform keeps,
/// and a street line built from them for everything that reads one.
fn address_from_elements(elements: &[(String, String)]) -> E911Address {
    let get = |name: &str| {
        elements
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    let civic = crate::phone_number::CivicElements {
        house_number: get("HNO"),
        house_number_suffix: get("HNS"),
        street_predirection: get("PRD"),
        street_name: get("RD"),
        street_suffix: get("STS"),
        street_postdirection: get("POD"),
        additional: get("LOC"),
        floor: get("FLR"),
        unit: get("UNIT"),
        room: get("ROOM"),
        building: get("BLD"),
        name: get("NAM"),
    };
    let street = [
        civic.house_number.as_deref(),
        civic.house_number_suffix.as_deref(),
        civic.street_predirection.as_deref(),
        civic.street_name.as_deref(),
        civic.street_suffix.as_deref(),
        civic.street_postdirection.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    E911Address {
        address1: street,
        address2: civic.additional.clone(),
        city: get("A3").unwrap_or_default(),
        state: get("A1").unwrap_or_default(),
        postal_code: get("PC").unwrap_or_default(),
        country: get("country").unwrap_or_default(),
        validated: false,
        validated_at: None,
        validated_by: None,
        provider_reference: None,
        civic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e911::LocationSource;
    use crate::phone_number::{CivicElements, E911Address};
    use quick_xml::events::Event;
    use quick_xml::Reader;

    /// RFC 5139's `civicAddress` sequence, in schema order.
    const CIVIC_SEQUENCE: [&str; 31] = [
        "country", "A1", "A2", "A3", "A4", "A5", "A6", "PRM", "PRD", "RD", "STS", "POD", "POM",
        "RDSEC", "RDBR", "RDSUBBR", "HNO", "HNS", "LMK", "LOC", "FLR", "NAM", "PC", "BLD", "UNIT",
        "ROOM", "SEAT", "PLC", "PCN", "POBOX", "ADDCODE",
    ];

    fn located(structured: bool, point: Option<GeoPoint>) -> DispatchableLocation {
        let civic = if structured {
            CivicElements {
                house_number: Some("1455".into()),
                house_number_suffix: Some("A".into()),
                street_predirection: Some("N".into()),
                street_name: Some("Market".into()),
                street_suffix: Some("St".into()),
                floor: Some("4".into()),
                room: Some("4B & <5>".into()),
                building: Some("East".into()),
                ..Default::default()
            }
        } else {
            CivicElements {
                floor: Some("3".into()),
                ..Default::default()
            }
        };
        DispatchableLocation {
            address: E911Address {
                address1: "1455A N Market St".into(),
                address2: Some("Floor 4".into()),
                city: "San Francisco".into(),
                state: "CA".into(),
                postal_code: "94103".into(),
                country: "US".into(),
                validated: true,
                validated_at: None,
                validated_by: None,
                provider_reference: None,
                civic,
            },
            callback_number: Some("+14155550100".into()),
            source: LocationSource::UserLocation,
            location_id: None,
            location_name: Some("Example HQ".into()),
            point,
            client_address: None,
            declared_age: None,
        }
    }

    fn subject() -> Subject<'static> {
        Subject {
            entity: "pres:+14155550100@pbx.example",
            device_id: "urn:uuid:6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_789_000_000),
        }
    }

    /// Every element as (qualified name, text), with the path of parents.
    fn elements(xml: &str) -> Vec<(Vec<String>, String)> {
        let mut reader = Reader::from_str(xml);
        let mut path: Vec<String> = Vec::new();
        let mut out = Vec::new();
        loop {
            match reader.read_event().expect("well-formed XML") {
                Event::Start(e) => {
                    path.push(String::from_utf8(e.name().as_ref().to_vec()).unwrap());
                    out.push((path.clone(), String::new()));
                }
                Event::Text(t) => {
                    let text = t.decode().unwrap().to_string();
                    if let Some(last) = out.last_mut() {
                        if last.0 == path {
                            last.1.push_str(&text);
                        }
                    }
                }
                Event::GeneralRef(r) => {
                    let text = match r.decode().unwrap().as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        other => panic!("unexpected entity {other}"),
                    };
                    if let Some(last) = out.last_mut() {
                        if last.0 == path {
                            last.1.push_str(text);
                        }
                    }
                }
                Event::End(_) => {
                    path.pop();
                }
                Event::Eof => break,
                _ => {}
            }
        }
        out
    }

    fn children<'a>(els: &'a [(Vec<String>, String)], parent: &[&str]) -> Vec<&'a str> {
        els.iter()
            .filter(|(p, _)| p.len() == parent.len() + 1 && p[..parent.len()] == *parent)
            .map(|(p, _)| p.last().unwrap().as_str())
            .collect()
    }

    fn text<'a>(els: &'a [(Vec<String>, String)], path: &[&str]) -> Option<&'a str> {
        els.iter().find(|(p, _)| p == path).map(|(_, t)| t.as_str())
    }

    #[test]
    fn a_structured_address_follows_the_rfc_5491_profile() {
        let point = GeoPoint {
            latitude: 37.7749,
            longitude: -122.4194,
        };
        let xml = document(&located(true, Some(point)), &subject());
        let els = elements(&xml);

        // Namespaces and the presentity.
        for ns in [
            PIDF_NS,
            DATA_MODEL_NS,
            GEOPRIV_NS,
            BASIC_POLICY_NS,
            CIVIC_NS,
            GML_NS,
        ] {
            assert!(xml.contains(&format!("\"{ns}\"")), "{ns} declared: {xml}");
        }
        assert!(xml.contains("entity=\"pres:+14155550100@pbx.example\""));

        // RFC 4479: the device's content, then its required deviceID, then
        // the timestamp; RFC 4119: location-info, usage-rules, method.
        assert_eq!(
            children(&els, &["presence", "dm:device"]),
            ["gp:geopriv", "gp:geopriv", "dm:deviceID", "dm:timestamp"]
        );
        // (Both geoprivs' children, in document order.)
        assert_eq!(
            children(&els, &["presence", "dm:device", "gp:geopriv"]),
            [
                "gp:location-info",
                "gp:usage-rules",
                "gp:method",
                "gp:location-info",
                "gp:usage-rules",
                "gp:method"
            ]
        );
        assert_eq!(
            text(&els, &["presence", "dm:device", "dm:timestamp"]),
            Some("2026-09-10T00:26:40Z")
        );
        assert_eq!(
            text(
                &els,
                &[
                    "presence",
                    "dm:device",
                    "gp:geopriv",
                    "gp:usage-rules",
                    "gbp:retransmission-allowed"
                ]
            ),
            Some("true")
        );

        // RFC 5139: the elements present, in the schema's order.
        let civic = [
            "presence",
            "dm:device",
            "gp:geopriv",
            "gp:location-info",
            "ca:civicAddress",
        ];
        let names: Vec<&str> = children(&els, &civic)
            .into_iter()
            .map(|n| n.trim_start_matches("ca:"))
            .collect();
        assert_eq!(
            names,
            [
                "country", "A1", "A3", "PRD", "RD", "STS", "HNO", "HNS", "FLR", "NAM", "PC", "BLD",
                "ROOM"
            ]
        );
        let positions: Vec<usize> = names
            .iter()
            .map(|n| CIVIC_SEQUENCE.iter().position(|s| s == n).unwrap())
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{names:?}");
        let at = |el: &str| {
            let mut path = civic.to_vec();
            path.push(el);
            text(&els, &path).map(str::to_string)
        };
        assert_eq!(at("ca:RD").as_deref(), Some("Market"));
        assert_eq!(at("ca:HNO").as_deref(), Some("1455"));
        assert_eq!(
            at("ca:ROOM").as_deref(),
            Some("4B & <5>"),
            "text is escaped and read back"
        );
        assert_eq!(at("ca:NAM").as_deref(), Some("Example HQ"));
        assert!(at("ca:LOC").is_none(), "the floor line is carried as FLR");

        // The point, in its own geopriv, latitude first.
        let pos = els
            .iter()
            .find(|(p, _)| p.last().map(String::as_str) == Some("gml:pos"))
            .unwrap();
        assert_eq!(pos.1, "37.7749 -122.4194");
        assert!(xml.contains("srsName=\"urn:ogc:def:crs:EPSG::4326\""));
    }

    #[test]
    fn an_unstructured_address_is_sent_with_its_street_line() {
        let xml = document(&located(false, None), &subject());
        let els = elements(&xml);
        let civic = [
            "presence",
            "dm:device",
            "gp:geopriv",
            "gp:location-info",
            "ca:civicAddress",
        ];
        let names = children(&els, &civic);
        assert!(!names.contains(&"ca:HNO"));
        let mut rd = civic.to_vec();
        rd.push("ca:RD");
        assert_eq!(text(&els, &rd), Some("1455A N Market St"));
        assert!(!xml.contains("gml"), "no coordinates, no point: {xml}");
        assert_eq!(
            children(&els, &["presence", "dm:device"])[..1],
            ["gp:geopriv"]
        );
    }

    #[test]
    fn a_document_this_platform_wrote_reads_back() {
        let point = GeoPoint {
            latitude: 37.7749,
            longitude: -122.4194,
        };
        let sent = located(true, Some(point));
        let read = parse(&document(&sent, &subject())).expect("parses").clone();
        let address = read.address.expect("a civic address");
        assert_eq!(address.address1, "1455 A N Market St");
        assert_eq!(address.city, "San Francisco");
        assert_eq!(address.state, "CA");
        assert_eq!(address.postal_code, "94103");
        assert_eq!(address.country, "US");
        assert_eq!(address.civic.house_number.as_deref(), Some("1455"));
        assert_eq!(address.civic.street_name.as_deref(), Some("Market"));
        assert_eq!(address.civic.room.as_deref(), Some("4B & <5>"));
        assert!(!address.validated, "a device's claim is never validated");
        assert_eq!(read.point, Some(point));
        assert_eq!(read.method.as_deref(), Some("Manual"));
    }

    #[test]
    fn another_stacks_prefixes_and_shapes_are_read() {
        // RFC 5491 §3.2's shape, with other prefixes, a tuple rather than a
        // device, and the point before the civic address.
        let xml = r#"<?xml version="1.0"?>
            <pres:presence xmlns:pres="urn:ietf:params:xml:ns:pidf"
                           xmlns:gp="urn:ietf:params:xml:ns:pidf:geopriv10"
                           xmlns:cl="urn:ietf:params:xml:ns:pidf:geopriv10:civicAddr"
                           xmlns:g="http://www.opengis.net/gml"
                           entity="pres:alice@example.com">
              <pres:tuple id="t1"><pres:status><gp:geopriv>
                <gp:location-info>
                  <g:Point srsName="urn:ogc:def:crs:EPSG::4326"><g:pos>-43.5723 153.2176</g:pos></g:Point>
                  <cl:civicAddress><cl:country>AU</cl:country><cl:A1>NSW</cl:A1>
                    <cl:A3>Wollongong</cl:A3><cl:RD>Flinders</cl:RD><cl:STS>St</cl:STS>
                    <cl:HNO>15</cl:HNO><cl:FLR>2</cl:FLR><cl:PC>2500</cl:PC></cl:civicAddress>
                </gp:location-info>
                <gp:usage-rules/>
                <gp:method>GPS</gp:method>
              </gp:geopriv></pres:status></pres:tuple>
            </pres:presence>"#;
        let read = parse(xml).expect("parses");
        let address = read.address.expect("a civic address");
        assert_eq!(address.address1, "15 Flinders St");
        assert_eq!(address.city, "Wollongong");
        assert_eq!(address.country, "AU");
        assert_eq!(address.civic.floor.as_deref(), Some("2"));
        assert_eq!(read.method.as_deref(), Some("GPS"));
        assert_eq!(
            read.point,
            Some(GeoPoint {
                latitude: -43.5723,
                longitude: 153.2176
            })
        );
    }

    #[test]
    fn nonsense_is_refused_or_empty() {
        assert!(parse("<presence>").is_none(), "unclosed XML");
        assert!(parse("not xml at all").unwrap().is_empty());
        // A document with no location this platform understands.
        let bare = r#"<presence xmlns="urn:ietf:params:xml:ns:pidf" entity="pres:a@b"/>"#;
        assert!(parse(bare).unwrap().is_empty());
        // A point off the globe is not a point.
        let off = r#"<presence xmlns="urn:ietf:params:xml:ns:pidf"
              xmlns:gp="urn:ietf:params:xml:ns:pidf:geopriv10"
              xmlns:gml="http://www.opengis.net/gml" entity="pres:a@b">
              <gp:geopriv><gp:location-info>
                <gml:Point><gml:pos>999 999</gml:pos></gml:Point>
              </gp:location-info><gp:usage-rules/></gp:geopriv></presence>"#;
        assert!(parse(off).unwrap().point.is_none());
    }
}
