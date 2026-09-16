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
                civic,
            },
            callback_number: Some("+14155550100".into()),
            source: LocationSource::UserLocation,
            location_id: None,
            location_name: Some("Example HQ".into()),
            point,
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
}
