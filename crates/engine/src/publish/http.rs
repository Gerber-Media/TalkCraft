//! HTTP for publishing, through the `curl` program every desktop system has (macOS, Linux, Windows
//! 10+): no network stack in the app. A `Transport` sends requests, so tests can check what would
//! be sent without a network.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    None,
    Json(Value),
    Form(Vec<(String, String)>),
    Multipart(Vec<(String, Part)>),
    /// The raw bytes of a file (PUT to a pre-signed upload URL).
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Request {
    pub fn new(method: &'static str, url: impl Into<String>) -> Self {
        Request { method, url: url.into(), headers: Vec::new(), body: Body::None }
    }
    pub fn header(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.to_string(), v.into()));
        self
    }
    pub fn json(mut self, v: Value) -> Self {
        self.body = Body::Json(v);
        self
    }
    pub fn form(mut self, f: Vec<(&str, String)>) -> Self {
        self.body = Body::Form(f.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        self
    }
    pub fn multipart(mut self, p: Vec<(&str, Part)>) -> Self {
        self.body = Body::Multipart(p.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
        self
    }
    pub fn file(mut self, p: PathBuf) -> Self {
        self.body = Body::File(p);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

pub trait Transport {
    fn send(&mut self, r: &Request) -> Result<Response, String>;
}

/// Sends a request and fails on an error status, with the start of the body as the reason.
pub fn call(t: &mut dyn Transport, r: Request) -> Result<Value, String> {
    let resp = t.send(&r)?;
    if resp.status >= 400 || resp.status == 0 {
        let snippet: String = resp.body.chars().take(300).collect();
        return Err(format!("{} {} → HTTP {}: {snippet}", r.method, strip_query(&r.url), resp.status));
    }
    Ok(resp.json())
}

/// A URL without its query (pre-signed URLs carry secrets there).
fn strip_query(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

/// The real transport: one `curl` per request.
pub struct Curl;

impl Transport for Curl {
    fn send(&mut self, r: &Request) -> Result<Response, String> {
        let mut c = Command::new("curl");
        c.args(["-sS", "--connect-timeout", "30", "--max-time", "7200", "-X", r.method, "-w", "\n%{http_code}"]);
        for (k, v) in &r.headers {
            c.arg("-H").arg(format!("{k}: {v}"));
        }
        match &r.body {
            Body::None => {}
            Body::Json(v) => {
                c.arg("-H").arg("Content-Type: application/json").arg("--data-binary").arg(v.to_string());
            }
            Body::Form(f) => {
                for (k, v) in f {
                    c.arg("--data-urlencode").arg(format!("{k}={v}"));
                }
            }
            Body::Multipart(parts) => {
                for (k, p) in parts {
                    match p {
                        // --form-string: a value starting with @ or < stays text.
                        Part::Text(v) => c.arg("--form-string").arg(format!("{k}={v}")),
                        Part::File(f) => c.arg("-F").arg(format!("{k}=@{}", f.display())),
                    };
                }
            }
            Body::File(f) => {
                c.arg("--upload-file").arg(f);
            }
        }
        c.arg(&r.url);
        let out = c.output().map_err(|e| format!("curl could not be started: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').unwrap_or(("", text.as_str()));
        let status: u16 = code.trim().parse().unwrap_or(0);
        if status == 0 {
            return Err(format!("{} {}: {}", r.method, strip_query(&r.url), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(Response { status, body: body.to_string() })
    }
}

/// A transport for tests: records requests and answers from a list, in order.
#[derive(Debug, Default)]
pub struct Fake {
    pub sent: Vec<Request>,
    pub answers: Vec<Response>,
}

impl Transport for Fake {
    fn send(&mut self, r: &Request) -> Result<Response, String> {
        self.sent.push(r.clone());
        if self.answers.is_empty() {
            return Ok(Response { status: 200, body: "{}".into() });
        }
        Ok(self.answers.remove(0))
    }
}

/// An answer for `Fake`.
pub fn ok(status: u16, body: Value) -> Response {
    Response { status, body: body.to_string() }
}
