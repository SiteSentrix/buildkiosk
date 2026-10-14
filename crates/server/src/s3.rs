//! Pushing a finished backup to object storage that speaks the S3 protocol (Amazon S3, MinIO, Backblaze B2,
//! Cloudflare R2, Wasabi and the like). Each file of the backup folder becomes one object under
//! `<prefix>backup-<seconds>/<path in the folder>`, signed with AWS Signature Version 4. Nothing is read
//! into memory whole: a file is hashed, then streamed.
//!
//! The access key and secret are never put in the journal or in the data directory: they come from the
//! environment of the process that starts the server.

use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// The most files one backup may hold when uploading.
const MAX_FILES: usize = 500_000;

/// Where and as whom backups are uploaded.
#[derive(Clone)]
pub struct S3Target {
    /// `https://s3.us-east-1.amazonaws.com`, `http://minio.lan:9000`: no path, no trailing slash.
    endpoint: String,
    bucket: String,
    region: String,
    /// Prepended to every key: `buildkiosk/` or empty.
    prefix: String,
    access_key: String,
    secret_key: String,
    /// `endpoint/bucket/key` (the default) instead of `bucket.endpoint/key`.
    path_style: bool,
}

impl S3Target {
    pub fn new(
        endpoint: &str,
        bucket: &str,
        region: &str,
        prefix: &str,
        access_key: &str,
        secret_key: &str,
        path_style: bool,
    ) -> Result<S3Target, String> {
        let endpoint = endpoint.trim().trim_end_matches('/').to_string();
        let rest = endpoint
            .strip_prefix("https://")
            .or_else(|| endpoint.strip_prefix("http://"))
            .ok_or("the endpoint must start with https:// (or http://)")?;
        if rest.is_empty() || rest.contains('/') || rest.contains('?') || rest.contains('@') {
            return Err("the endpoint should be just a host, such as https://s3.us-east-1.amazonaws.com".into());
        }
        let ok_bucket =
            |b: &str| (3..=63).contains(&b.len()) && b.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.');
        if !ok_bucket(bucket) {
            return Err("the bucket name should be 3 to 63 lower-case letters, digits, dots or hyphens".into());
        }
        if region.is_empty() || !region.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
            return Err("the region should be letters, digits and hyphens, such as us-east-1".into());
        }
        if access_key.is_empty() || secret_key.is_empty() {
            return Err("the access key and secret key are needed (BUILDKIOSK_S3_ACCESS_KEY and BUILDKIOSK_S3_SECRET_KEY)".into());
        }
        let mut prefix = prefix.trim().trim_start_matches('/').to_string();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        Ok(S3Target {
            endpoint,
            bucket: bucket.to_string(),
            region: region.to_string(),
            prefix,
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            path_style,
        })
    }

    /// Where the backup is described on the backups page.
    pub fn describe(&self) -> String {
        format!("{}/{}/{}", self.endpoint, self.bucket, self.prefix)
    }

    /// The URL and the `Host` header value for an object key.
    fn locate(&self, key: &str) -> (String, String, String) {
        let (scheme, host) = match self.endpoint.split_once("://") {
            Some((s, h)) => (s, h),
            None => ("https", self.endpoint.as_str()),
        };
        let encoded = uri_encode(key);
        if self.path_style {
            let path = format!("/{}/{encoded}", self.bucket);
            (format!("{scheme}://{host}{path}"), host.to_string(), path)
        } else {
            let host = format!("{}.{host}", self.bucket);
            let path = format!("/{encoded}");
            (format!("{scheme}://{host}{path}"), host, path)
        }
    }

    /// Uploads every file under `dir` as `<prefix><name>/<relative path>`. Returns how many files went up.
    pub fn upload_folder(&self, dir: &Path, name: &str, now_secs: u64) -> Result<usize, String> {
        let files = list_files(dir)?;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(300))
            .timeout_write(Duration::from_secs(900))
            .build();
        for (path, rel) in &files {
            self.put_file(&agent, path, &format!("{}{name}/{rel}", self.prefix), now_secs)?;
        }
        Ok(files.len())
    }

    fn put_file(&self, agent: &ureq::Agent, path: &Path, key: &str, now_secs: u64) -> Result<(), String> {
        let (hash, size) = hash_file(path)?;
        let (url, host, canonical_path) = self.locate(key);
        let amz_date = amz_date(now_secs);
        let mut headers =
            vec![("host".to_string(), host), ("x-amz-content-sha256".to_string(), hash.clone()), ("x-amz-date".to_string(), amz_date.clone())];
        let auth = authorization("PUT", &canonical_path, &mut headers, &hash, &self.access_key, &self.secret_key, &self.region, &amz_date);
        let file = std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let res = agent
            .put(&url)
            .set("x-amz-content-sha256", &hash)
            .set("x-amz-date", &amz_date)
            .set("authorization", &auth)
            .set("content-length", &size.to_string())
            .send(file);
        match res {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, r)) => {
                // The service explains itself in a short XML body: keep the beginning of it.
                let mut body = String::new();
                let _ = r.into_reader().take(400).read_to_string(&mut body);
                Err(format!("the storage service refused {key}: {code} {}", body.split_whitespace().collect::<Vec<_>>().join(" ")))
            }
            Err(e) => Err(format!("cannot reach the storage service for {key}: {e}")),
        }
    }
}

/// Every file under `dir` with its path relative to `dir` (using `/`), in a stable order.
fn list_files(dir: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out: Vec<(PathBuf, String)> = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, rel)) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("cannot read {}: {e}", d.display()))?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel_path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push((e.path(), rel_path)),
                Ok(t) if t.is_file() => out.push((e.path(), rel_path)),
                _ => {}
            }
            if out.len() > MAX_FILES {
                return Err(format!("the backup holds more than {MAX_FILES} files"));
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

fn hash_file(path: &Path) -> Result<(String, u64), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let (mut h, mut buf, mut size) = (Sha256::new(), vec![0u8; 256 * 1024], 0u64);
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or(&[]));
        size = size.saturating_add(n as u64);
    }
    Ok((hex(&h.finalize()), size))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A key as it appears in the URL: everything but letters, digits and `-_.~` is %-escaped, `/` stays.
fn uri_encode(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `20260512T091500Z` for a time in unix seconds.
fn amz_date(secs: u64) -> String {
    let days = i32::try_from(secs / 86_400).unwrap_or(0);
    let (y, m, d) = buildkiosk_core::Date::from_days(days).map_or((1970, 1, 1), |d| d.ymd());
    let rest = secs % 86_400;
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    match Hmac::<Sha256>::new_from_slice(key) {
        Ok(mut m) => {
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
        // An HMAC accepts a key of any length, so this is never reached; an empty answer fails the signature rather than a request.
        Err(_) => Vec::new(),
    }
}

/// The `Authorization` header of an S3 request (AWS Signature Version 4). `headers` are the ones to sign (names in
/// lower case, `host` among them); they are put in order here.
#[allow(clippy::too_many_arguments)]
pub fn authorization(
    method: &str,
    canonical_path: &str,
    headers: &mut [(String, String)],
    payload_hash: &str,
    access_key: &str,
    secret_key: &str,
    region: &str,
    amz_date: &str,
) -> String {
    headers.sort();
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical_request = format!("{method}\n{canonical_path}\n\n{canonical_headers}\n{signed}\n{payload_hash}");
    let day = amz_date.get(..8).unwrap_or("");
    let scope = format!("{day}/{region}/s3/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex(&Sha256::digest(canonical_request.as_bytes())));
    let key = [day, region, "s3", "aws4_request"].iter().fold(format!("AWS4{secret_key}").into_bytes(), |k, part| hmac(&k, part.as_bytes()));
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={signature}")
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    /// The worked example in the published S3 signing guide: uploading "Welcome to Amazon S3." as `test$file.text`.
    #[test]
    fn signing_matches_the_published_example() {
        let hash = hex(&Sha256::digest(b"Welcome to Amazon S3."));
        assert_eq!(hash, "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072");
        let mut headers = vec![
            ("date".to_string(), "Fri, 24 May 2013 00:00:00 GMT".to_string()),
            ("host".to_string(), "examplebucket.s3.amazonaws.com".to_string()),
            ("x-amz-content-sha256".to_string(), hash.clone()),
            ("x-amz-date".to_string(), "20130524T000000Z".to_string()),
            ("x-amz-storage-class".to_string(), "REDUCED_REDUNDANCY".to_string()),
        ];
        let auth = authorization(
            "PUT",
            "/test%24file.text",
            &mut headers,
            &hash,
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "20130524T000000Z",
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class, \
             Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }

    #[test]
    fn keys_urls_and_dates() {
        assert_eq!(uri_encode("buildkiosk/backup-1/data/a b$c.txt"), "buildkiosk/backup-1/data/a%20b%24c.txt");
        assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
        assert_eq!(amz_date(1_788_000_000 + 3_661), amz_date(1_788_003_661));
        let t = S3Target::new("https://s3.example.com/", "my-bucket", "us-east-1", "bk", "a", "b", true).unwrap();
        assert_eq!(
            t.locate("bk/x y"),
            ("https://s3.example.com/my-bucket/bk/x%20y".to_string(), "s3.example.com".to_string(), "/my-bucket/bk/x%20y".to_string())
        );
        let v = S3Target::new("http://minio.lan:9000", "my-bucket", "us-east-1", "", "a", "b", false).unwrap();
        assert_eq!(v.locate("k").1, "my-bucket.minio.lan:9000");
        assert_eq!(t.prefix, "bk/");
        for bad in [
            ("ftp://x", "bucket"),
            ("https://x/path", "bucket"),
            ("https://x", "UPPER"),
            ("https://x", "ab"),
            ("https://u@x", "bucket"),
            ("", "bucket"),
        ] {
            assert!(S3Target::new(bad.0, bad.1, "us-east-1", "", "a", "b", true).is_err(), "{bad:?}");
        }
        assert!(S3Target::new("https://x", "bucket", "us east", "", "a", "b", true).is_err());
        assert!(S3Target::new("https://x", "bucket", "us-east-1", "", "", "b", true).is_err());
    }

    /// A request the test service received: the path, the headers (names in lower case) and the body.
    pub(crate) type Put = (String, Vec<(String, String)>, Vec<u8>);

    /// A tiny storage service on a local port that records every PUT and answers as asked.
    pub(crate) fn serve(status: &'static str) -> (String, std::sync::mpsc::Receiver<Put>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut out = stream;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                        let mut headers = Vec::new();
                        loop {
                            let mut h = String::new();
                            if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                                break;
                            }
                            if let Some((k, v)) = h.trim().split_once(':') {
                                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                            }
                        }
                        let len = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0usize);
                        let mut body = vec![0u8; len];
                        reader.read_exact(&mut body).unwrap();
                        tx.send((path, headers, body)).unwrap();
                        let msg = if status.starts_with("200") { "" } else { "<Error><Code>AccessDenied</Code></Error>" };
                        write!(out, "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{msg}", msg.len()).unwrap();
                    }
                });
            }
        });
        (format!("http://{addr}"), rx)
    }

    #[test]
    fn a_backup_folder_goes_up_file_by_file() {
        let dir = std::env::temp_dir().join(format!("buildkiosk-s3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("acme/blobs")).unwrap();
        std::fs::write(dir.join("instance.json"), b"{}").unwrap();
        std::fs::write(dir.join("acme/journal.jsonl"), b"line one\nline two\n").unwrap();
        std::fs::write(dir.join("acme/blobs/a b"), vec![7u8; 300_000]).unwrap();

        let (endpoint, rx) = serve("200 OK");
        let t = S3Target::new(&endpoint, "bucket", "us-east-1", "bk", "AK", "SK", true).unwrap();
        assert_eq!(t.upload_folder(&dir, "backup-1788003661", 1_788_003_661).unwrap(), 3);
        let mut got: Vec<_> = rx.try_iter().collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        let paths: Vec<&str> = got.iter().map(|g| g.0.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/bucket/bk/backup-1788003661/acme/blobs/a%20b",
                "/bucket/bk/backup-1788003661/acme/journal.jsonl",
                "/bucket/bk/backup-1788003661/instance.json"
            ]
        );
        let (_, headers, body) = &got[0];
        assert_eq!(body.len(), 300_000);
        let header = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(header("x-amz-content-sha256"), hex(&Sha256::digest(body)));
        assert_eq!(header("x-amz-date"), "20260829T114101Z");
        assert!(header("authorization").starts_with(
            "AWS4-HMAC-SHA256 Credential=AK/20260829/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="
        ));
        // The secret never travels.
        assert!(!headers.iter().any(|(_, v)| v.contains("SK")));

        // A refusal is reported with the service's own words; an unreachable service too.
        let (endpoint, _rx) = serve("403 Forbidden");
        let t = S3Target::new(&endpoint, "bucket", "us-east-1", "", "AK", "SK", true).unwrap();
        let e = t.upload_folder(&dir, "backup-1", 1_788_003_661).unwrap_err();
        assert!(e.contains("403") && e.contains("AccessDenied"), "{e}");
        let t = S3Target::new("http://127.0.0.1:9", "bucket", "us-east-1", "", "AK", "SK", true).unwrap();
        assert!(t.upload_folder(&dir, "backup-1", 1_788_003_661).unwrap_err().contains("cannot reach"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
