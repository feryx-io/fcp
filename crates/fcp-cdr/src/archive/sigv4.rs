//! AWS Signature Version 4 for S3 requests, header-based, with the payload
//! hash signed (`x-amz-content-sha256`): what an S3-compatible store needs
//! to accept a PUT, and to verify the body it received is the body signed.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// The hash of an empty payload.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Static credentials.
#[derive(Clone)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Percent-encode as SigV4 wants: everything but `A-Za-z0-9-._~`, and `/`
/// too unless `keep_slash`.
pub fn uri_encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// A request to sign. `path` is the URI path as sent, already encoded;
/// `query` the decoded pairs; `headers` every header to sign, `host`
/// included, names in any case.
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a [(String, String)],
    pub headers: &'a [(String, String)],
    pub payload_sha256: &'a str,
}

/// The `Authorization` header value for `request`.
pub fn authorization(
    request: &Request<'_>,
    credentials: &Credentials,
    region: &str,
    service: &str,
    now: DateTime<Utc>,
) -> String {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();

    let mut headers: Vec<(String, String)> = request
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k.to_ascii_lowercase(),
                v.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect();
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let mut query: Vec<(String, String)> = request
        .query
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        request.method,
        request.path,
        canonical_query,
        canonical_headers,
        signed_headers,
        request.payload_sha256
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac(
        format!("AWS4{}", credentials.secret_access_key).as_bytes(),
        &date,
    );
    let k_region = hmac(&k_date, region);
    let k_service = hmac(&k_region, service);
    let k_signing = hmac(&k_service, "aws4_request");
    let signature = hex::encode(hmac(&k_signing, &string_to_sign));

    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // The examples in the S3 API reference, "Signature Calculations for the
    // Authorization Header: Transferring Payload in a Single Chunk".
    fn example() -> (Credentials, DateTime<Utc>) {
        (
            Credentials {
                access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
                secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
                session_token: None,
            },
            Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap(),
        )
    }

    fn signature(auth: &str) -> &str {
        auth.rsplit("Signature=").next().unwrap()
    }

    #[test]
    fn get_object_example() {
        let (creds, now) = example();
        let headers = vec![
            ("Host".into(), "examplebucket.s3.amazonaws.com".into()),
            ("Range".into(), "bytes=0-9".into()),
            ("x-amz-content-sha256".into(), EMPTY_SHA256.into()),
            ("x-amz-date".into(), "20130524T000000Z".into()),
        ];
        let auth = authorization(
            &Request {
                method: "GET",
                path: "/test.txt",
                query: &[],
                headers: &headers,
                payload_sha256: EMPTY_SHA256,
            },
            &creds,
            "us-east-1",
            "s3",
            now,
        );
        assert_eq!(
            signature(&auth),
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        assert!(auth.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, "
        ));
    }

    #[test]
    fn put_object_example() {
        let (creds, now) = example();
        let body = b"Welcome to Amazon S3.";
        let hash = sha256_hex(body);
        assert_eq!(
            hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let headers = vec![
            ("Host".into(), "examplebucket.s3.amazonaws.com".into()),
            ("Date".into(), "Fri, 24 May 2013 00:00:00 GMT".into()),
            ("x-amz-date".into(), "20130524T000000Z".into()),
            ("x-amz-storage-class".into(), "REDUCED_REDUNDANCY".into()),
            ("x-amz-content-sha256".into(), hash.clone()),
        ];
        let auth = authorization(
            &Request {
                method: "PUT",
                path: &format!("/{}", uri_encode("test$file.text", true)),
                query: &[],
                headers: &headers,
                payload_sha256: &hash,
            },
            &creds,
            "us-east-1",
            "s3",
            now,
        );
        assert_eq!(
            signature(&auth),
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn list_objects_example_signs_the_query() {
        let (creds, now) = example();
        let headers = vec![
            ("Host".into(), "examplebucket.s3.amazonaws.com".into()),
            ("x-amz-content-sha256".into(), EMPTY_SHA256.into()),
            ("x-amz-date".into(), "20130524T000000Z".into()),
        ];
        let auth = authorization(
            &Request {
                method: "GET",
                path: "/",
                query: &[
                    ("prefix".into(), "J".into()),
                    ("max-keys".into(), "2".into()),
                ],
                headers: &headers,
                payload_sha256: EMPTY_SHA256,
            },
            &creds,
            "us-east-1",
            "s3",
            now,
        );
        assert_eq!(
            signature(&auth),
            "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
        );
    }

    #[test]
    fn encoding_keeps_only_unreserved_characters() {
        assert_eq!(uri_encode("a b/c+d~e", true), "a%20b/c%2Bd~e");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(uri_encode("é", false), "%C3%A9");
    }

    #[test]
    fn credentials_do_not_print_their_secret() {
        let (creds, _) = example();
        let shown = format!("{creds:?}");
        assert!(!shown.contains("wJalr"));
    }
}
