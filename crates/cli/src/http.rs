//! HTTP plumbing shared by every online subcommand: a no-proxy `ureq`
//! agent, JSON verb helpers, the `--project` routing header, and the
//! small `ServerOpt` / `print_json` conveniences the handlers lean on.

use anyhow::{Context, Result};

/// The shared `--server URL` option, flattened into every online
/// subcommand so the flag name and default stay identical everywhere.
#[derive(clap::Args, Debug)]
pub(crate) struct ServerOpt {
    /// Base URL of the IA2 HTTP server to talk to. Defaults to the
    /// local server; point it at an edge box (e.g. via an SSH-forwarded
    /// port) to reach a remote runtime.
    #[arg(long, default_value = "http://127.0.0.1:3001")]
    pub server: String,
}

/// Pretty-print a serialisable value as JSON on stdout and return the
/// clean-success exit code. Collapses the `println!("{}",
/// serde_json::to_string_pretty(&v)?); Ok(0)` tail every online
/// subcommand handler ends with.
pub(crate) fn print_json<T: serde::Serialize>(v: &T) -> Result<i32> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(0)
}

/// Shared helper: read a JSON document from a file path, or from
/// stdin if `from == "-"`. Used by every `set --from` subcommand
/// so the shape is consistent (matches what `cs pou save` already
/// does for source text).
pub(crate) fn read_json_blob(from: &str) -> Result<serde_json::Value> {
    use std::io::Read;
    let bytes = if from == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf.into_bytes()
    } else {
        std::fs::read(from).with_context(|| format!("reading {from}"))?
    };
    serde_json::from_slice(&bytes).with_context(|| format!("parsing JSON from {from}"))
}

pub(crate) fn put_json(url: &str, body: &impl serde::Serialize) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().put(url))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow::anyhow!("PUT {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}

/// Tiny URL-component escaper. Variable names should be IEC identifiers
/// (alphanumeric + `_`), but operators sometimes write `instance.pin`
/// in `cs runtime force foo.bar` — the dot is safe but slashes
/// wouldn't be. Cover the common cases without pulling a full
/// percent-encoding crate.
pub(crate) fn url_encode(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '~') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

/// Build a no-proxy ureq Agent. ureq 2.x auto-picks up `HTTP_PROXY` /
/// `HTTPS_PROXY` env vars at request time, which routes our localhost
/// API traffic through the user's developer proxy (Clash etc.). Users
/// running a system-wide proxy see "Header field didn't end with \n"
/// because their proxy speaks SOCKS / Trojan, not HTTP. Building an
/// explicit Agent with no proxy fixes it.
///
/// We cache the Agent in a OnceLock so each `cs` invocation pays the
/// build cost once even if it makes several requests.
pub(crate) fn http_agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            // No `.proxy(...)` call — ureq treats absence as "direct
            // connection". Without this Agent, the static `ureq::post(...)`
            // path defaults to reading proxy from env.
            .timeout(std::time::Duration::from_secs(30))
            .build()
    })
}

/// Stores the `--project NAME` value parsed off the command line.
/// When present, every HTTP request adds an `X-IA2-Project` header
/// so the server routes the call to the named project; otherwise
/// the header is omitted and the server uses its active fallback
/// (back-compat with all the existing single-window flows).
pub(crate) static PROJECT_OVERRIDE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Wrap a `ureq::Request` so it carries the X-IA2-Project header when
/// the user passed `--project NAME`. The builder pattern means each
/// call site is a one-line `with_project_header(http_agent().post(url))`
/// or similar.
fn with_project_header(req: ureq::Request) -> ureq::Request {
    if let Some(name) = PROJECT_OVERRIDE.get() {
        req.set("X-IA2-Project", name)
    } else {
        req
    }
}

pub(crate) fn post_json(url: &str, body: &impl serde::Serialize) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().post(url))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}

pub(crate) fn get_json(url: &str) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().get(url))
        .call()
        .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}

pub(crate) fn delete_json(url: &str) -> Result<serde_json::Value> {
    let resp = with_project_header(http_agent().delete(url))
        .call()
        .map_err(|e| anyhow::anyhow!("DELETE {url}: {e}"))?;
    let value: serde_json::Value = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("decode JSON from {url}: {e}"))?;
    Ok(value)
}
