//! AWS Signature Version 4 for one request (the headers that sign it), as AWS documents it:
//! <https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html>.

use chrono::{DateTime, Utc};
use ring::{digest, hmac};

use super::credentials::Keys;

/// What gets signed: the request as it goes on the wire.
pub(super) struct Request<'a> {
    pub method: &'a str,
    /// `host[:port]`, as the `Host` header carries it.
    pub host: &'a str,
    /// The path as sent (each segment already URI-encoded once).
    pub path: &'a str,
    pub payload: &'a [u8],
    pub content_type: Option<&'a str>,
}

/// Where the signature is good: the region and the service (`bedrock` for Bedrock Runtime).
pub(super) struct Scope<'a> {
    pub region: &'a str,
    pub service: &'a str,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, data).as_ref())
}

fn mac(key: &[u8], data: &str) -> Vec<u8> {
    let k = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&k, data.as_bytes()).as_ref().to_vec()
}

/// `s` URI-encoded as SigV4 wants: every byte but `A-Z a-z 0-9 - . _ ~` as `%XX`.
pub(super) fn uri_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The canonical URI of a path sent to any service but S3: each segment encoded once more.
fn canonical_uri(path: &str) -> String {
    let segments: Vec<String> = path.split('/').map(uri_encode).collect();
    segments.join("/")
}

/// The headers to sign, lower-case and sorted by name.
fn signed_headers(req: &Request<'_>, keys: &Keys, amz_date: &str) -> Vec<(&'static str, String)> {
    let mut h = vec![
        ("host", req.host.to_string()),
        ("x-amz-date", amz_date.into()),
    ];
    if let Some(ct) = req.content_type {
        h.push(("content-type", ct.into()));
    }
    if let Some(token) = &keys.session_token {
        h.push(("x-amz-security-token", token.expose().into()));
    }
    h.sort_by_key(|(k, _)| *k);
    h
}

/// The canonical request: method, URI, (empty) query, the signed headers and the payload's hash.
fn canonical_request(req: &Request<'_>, headers: &[(&str, String)], names: &str) -> String {
    let listed: String = headers
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    format!(
        "{}\n{}\n\n{listed}\n{names}\n{}",
        req.method,
        canonical_uri(req.path),
        sha256_hex(req.payload)
    )
}

/// The key of `date`'s signatures in `scope`.
fn signing_key(keys: &Keys, date: &str, scope: &Scope<'_>) -> Vec<u8> {
    let first = format!("AWS4{}", keys.secret.expose()).into_bytes();
    [date, scope.region, scope.service, "aws4_request"]
        .iter()
        .fold(first, |k, part| mac(&k, part))
}

/// The headers to add to `req` so AWS accepts it: `x-amz-date`, `authorization` and, for
/// temporary credentials, `x-amz-security-token`.
pub(super) fn sign(
    req: &Request<'_>,
    keys: &Keys,
    scope: &Scope<'_>,
    now: DateTime<Utc>,
) -> Vec<(&'static str, String)> {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = &amz_date[..8];
    let headers = signed_headers(req, keys, &amz_date);
    let names: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
    let names = names.join(";");
    let canonical = canonical_request(req, &headers, &names);
    let credential_scope = format!("{date}/{}/{}/aws4_request", scope.region, scope.service);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let signature = hex(&mac(&signing_key(keys, date, scope), &to_sign));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={names}, Signature={signature}",
        keys.access_key
    );
    let mut out: Vec<(&'static str, String)> = headers
        .into_iter()
        .filter(|(k, _)| *k != "host" && *k != "content-type")
        .collect();
    out.push(("authorization", authorization));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Secret;
    use chrono::TimeZone;

    fn example_keys() -> Keys {
        Keys {
            access_key: "AKIDEXAMPLE".into(),
            secret: Secret::new("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"),
            session_token: None,
            expires: None,
        }
    }

    /// AWS's own SigV4 test suite, case `get-vanilla`: `GET /` to `example.amazonaws.com`,
    /// service `service`, us-east-1, 2015-08-30T12:36:00Z.
    #[test]
    fn signs_aws_get_vanilla_example() {
        let req = Request {
            method: "GET",
            host: "example.amazonaws.com",
            path: "/",
            payload: b"",
            content_type: None,
        };
        let scope = Scope {
            region: "us-east-1",
            service: "service",
        };
        let now = Utc.with_ymd_and_hms(2015, 8, 30, 12, 36, 0).unwrap();
        let h = sign(&req, &example_keys(), &scope, now);
        let auth = &h.iter().find(|(k, _)| *k == "authorization").unwrap().1;
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
        assert!(h.contains(&("x-amz-date", "20150830T123600Z".into())));
    }

    #[test]
    fn a_model_id_is_encoded_twice_in_the_canonical_uri() {
        let path = format!("/model/{}/converse", uri_encode("us.anthropic.claude-x:0"));
        assert_eq!(path, "/model/us.anthropic.claude-x%3A0/converse");
        assert_eq!(
            canonical_uri(&path),
            "/model/us.anthropic.claude-x%253A0/converse"
        );
    }

    #[test]
    fn temporary_credentials_sign_their_token() {
        let keys = Keys {
            session_token: Some(Secret::new("tok")),
            ..example_keys()
        };
        let req = Request {
            method: "POST",
            host: "bedrock-runtime.us-east-1.amazonaws.com",
            path: "/model/m/converse",
            payload: b"{}",
            content_type: Some("application/json"),
        };
        let scope = Scope {
            region: "us-east-1",
            service: "bedrock",
        };
        let h = sign(&req, &keys, &scope, Utc::now());
        let auth = &h.iter().find(|(k, _)| *k == "authorization").unwrap().1;
        assert!(auth.contains("SignedHeaders=content-type;host;x-amz-date;x-amz-security-token"));
        assert!(h.contains(&("x-amz-security-token", "tok".into())));
    }
}
