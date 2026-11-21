//! `cs-vision`: ask the local vision model when a photo was taken.
//!
//! The model runs in llama.cpp's `llama-server`, started by us as a child
//! process with:
//! * `--host 127.0.0.1` and a random free port: nothing outside this PC can
//!   connect;
//! * `--offline`: llama.cpp never downloads anything (we also never pass `-hf`);
//! * a random API key (passed in a file, not on the command line, so other
//!   programs cannot read it from the process list): other local programs
//!   cannot use the server;
//! * `--no-webui`: no browser interface.
//!
//! The server is stopped when [`Engine`] is dropped. Photos are sent only to
//! this local process, as a downscaled JPEG.

use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Error(pub String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
fn e(x: impl std::fmt::Display) -> Error {
    Error(x.to_string())
}

pub struct EngineFiles<'a> {
    /// `llama-server(.exe)`
    pub server: &'a Path,
    pub model: &'a Path,
    pub projector: &'a Path,
    /// Folder for the server log and the short-lived key file.
    pub work_dir: &'a Path,
}

/// Owned version of [`EngineFiles`].
#[derive(Debug, Clone)]
pub struct EngineFilesOwned {
    pub server: PathBuf,
    pub model: PathBuf,
    pub projector: PathBuf,
    pub work_dir: PathBuf,
}

impl EngineFilesOwned {
    pub fn as_ref(&self) -> EngineFiles<'_> {
        EngineFiles { server: &self.server, model: &self.model, projector: &self.projector, work_dir: &self.work_dir }
    }
}

pub struct Engine {
    child: Child,
    /// Ends the server even if ChronoSort crashes.
    _guard: cs_platform::ChildGuard,
    base: String,
    key: String,
    agent: ureq::Agent,
    pub log: PathBuf,
}

/// 128 random bits as hex, from the OS-seeded hasher keys (no extra crate).
fn random_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..2)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()),
            );
            h.write_u32(std::process::id() ^ i);
            format!("{:016x}", h.finish())
        })
        .collect()
}

fn free_port() -> Result<u16, Error> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0").map_err(e)?.local_addr().map_err(e)?.port())
}

impl Engine {
    /// Start the server and wait until the model is loaded (`ready_timeout`).
    pub fn start(files: &EngineFiles, gpu: bool, ready_timeout: Duration) -> Result<Self, Error> {
        std::fs::create_dir_all(files.work_dir).map_err(e)?;
        let key = random_key();
        let key_file = files.work_dir.join(format!("vision-{}.key", std::process::id()));
        std::fs::File::create(&key_file).and_then(|mut f| f.write_all(key.as_bytes())).map_err(e)?;
        let log = files.work_dir.join("vision-server.log");
        let log_file = std::fs::File::create(&log).map_err(e)?;
        let port = free_port()?;
        let mut cmd = Command::new(files.server);
        cmd.arg("--model")
            .arg(files.model)
            .arg("--mmproj")
            .arg(files.projector)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["--offline", "--no-webui", "--ctx-size", "8192", "--parallel", "1"])
            .args(["--reasoning", "off"])
            .args(["--n-gpu-layers", if gpu { "999" } else { "0" }])
            .arg("--api-key-file")
            .arg(&key_file)
            .stdin(Stdio::null())
            .stdout(log_file.try_clone().map_err(e)?)
            .stderr(log_file);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cs_platform::prepare_child(&mut cmd);
        let mut child = cmd.spawn().map_err(|x| Error(format!("could not start the vision engine: {x}")))?;
        let guard = match cs_platform::end_with_us(&child) {
            Ok(g) => g,
            Err(x) => {
                let _ = child.kill();
                return Err(Error(format!("could not tie the vision engine to ChronoSort: {x}")));
            }
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(300)))
            .http_status_as_error(false)
            .build()
            .into();
        let mut eng = Engine { child, _guard: guard, base: format!("http://127.0.0.1:{port}"), key, agent, log };
        let res = eng.wait_ready(ready_timeout);
        let _ = std::fs::remove_file(&key_file); // read at start-up; not needed any more
        res.map(|()| eng)
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<(), Error> {
        let t = Instant::now();
        loop {
            if let Some(st) = self.child.try_wait().map_err(e)? {
                let tail = std::fs::read_to_string(&self.log).unwrap_or_default();
                let tail: String =
                    tail.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                return Err(Error(format!("the vision engine stopped ({st}):\n{tail}")));
            }
            if let Ok(r) = self.agent.get(format!("{}/health", self.base)).call()
                && r.status() == 200
            {
                return Ok(());
            }
            if t.elapsed() > timeout {
                return Err(Error("the vision engine did not get ready in time".into()));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// One chat request with an image; `schema` constrains the answer to JSON.
    pub fn ask_json(&self, prompt: &str, jpeg: &[u8], schema: &serde_json::Value) -> Result<serde_json::Value, Error> {
        let body = serde_json::json!({
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{}", base64(jpeg))}},
                    {"type": "text", "text": prompt},
                ],
            }],
            "temperature": 0.1,
            "max_tokens": 400,
            "response_format": {"type": "json_object", "schema": schema},
        });
        let mut r = self
            .agent
            .post(format!("{}/v1/chat/completions", self.base))
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send(&serde_json::to_vec(&body).map_err(e)?[..])
            .map_err(e)?;
        let status = r.status();
        let text = r.body_mut().with_config().limit(16 << 20).read_to_string().map_err(e)?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|x| Error(format!("{x}: {text}")))?;
        if status != 200 {
            return Err(Error(format!("vision engine error {status}: {v}")));
        }
        let text = v["choices"][0]["message"]["content"].as_str().ok_or("no answer").map_err(e)?;
        serde_json::from_str(text).map_err(|x| Error(format!("unreadable answer ({x}): {text}")))
    }

    /// Ask when the photo was taken.
    pub fn date_photo(&self, jpeg: &[u8]) -> Result<DateGuess, Error> {
        let v = self.ask_json(DATE_PROMPT, jpeg, &date_schema())?;
        let g: DateGuess = serde_json::from_value(v).map_err(e)?;
        Ok(g.sane())
    }
}

impl Engine {
    /// Estimated age in years of the (one) person in a face picture.
    pub fn estimate_age(&self, jpeg: &[u8]) -> Result<i32, Error> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"age": {"type": "integer"}},
            "required": ["age"]
        });
        let v = self.ask_json(AGE_PROMPT, jpeg, &schema)?;
        let age = v["age"].as_i64().ok_or_else(|| Error("no age in the answer".into()))?;
        Ok(age.clamp(0, 110) as i32)
    }
}

pub const AGE_PROMPT: &str = "How old is the person in this picture? Estimate their age in years from the face only (skin, hair, wrinkles, face shape). Do not say who they are. Answer with the age as a number.";

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub const DATE_PROMPT: &str = "You help sort a family photo collection by year. Estimate when this photo was taken, using only what you can see: a date printed or burned into the picture (often orange digits in a corner), the photo or print technology (colour cast, fading, borders, film or digital look), clothing, hairstyles, cars, devices, buildings and signs. Do not say or guess who the people are, and do not rely on recognising famous people or events: describe objects and styles only. If a date is printed in the picture, copy it exactly into date_stamp, otherwise use null. Give your single best year and up to three short clues.";

/// How far off the best year typically is (measured with Qwen3-VL 8B on 63
/// dated photos, 2026-11-21: ±5 years held for 70 %, ±8 for 87 %).
pub const TYPICAL_ERROR_YEARS: i32 = 5;

fn date_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "date_stamp": {"type": ["string", "null"]},
            "best_year": {"type": "integer"},
            "clues": {"type": "array", "items": {"type": "string"}, "maxItems": 3}
        },
        "required": ["date_stamp", "best_year", "clues"]
    })
}

/// The model's estimate. The range is not the model's own (it was far too
/// sure of itself): it is best_year ± [`TYPICAL_ERROR_YEARS`], as measured.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DateGuess {
    pub date_stamp: Option<String>,
    pub best_year: i32,
    #[serde(default)]
    pub earliest_year: i32,
    #[serde(default)]
    pub latest_year: i32,
    pub clues: Vec<String>,
}

impl DateGuess {
    /// Year inside 1826 (first photo) ..= 2100, measured range, at most 3 clues.
    fn sane(mut self) -> Self {
        self.best_year = self.best_year.clamp(1826, 2100);
        self.earliest_year = (self.best_year - TYPICAL_ERROR_YEARS).max(1826);
        self.latest_year = (self.best_year + TYPICAL_ERROR_YEARS).min(2100);
        self.clues.truncate(3);
        self
    }
}

/// Standard base64 (RFC 4648, with padding).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let n =
            (u32::from(ch[0]) << 16) | (u32::from(*ch.get(1).unwrap_or(&0)) << 8) | u32::from(*ch.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= ch.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn guesses_are_made_sane() {
        let g = DateGuess {
            date_stamp: None,
            best_year: 1500,
            earliest_year: 0,
            latest_year: 0,
            clues: vec!["a".into(); 5],
        }
        .sane();
        assert_eq!((g.earliest_year, g.latest_year, g.best_year, g.clues.len()), (1826, 1831, 1826, 3));
        let g = DateGuess { date_stamp: None, best_year: 1985, earliest_year: 0, latest_year: 0, clues: vec![] }.sane();
        assert_eq!((g.earliest_year, g.latest_year), (1980, 1990));
    }

    #[test]
    fn keys_differ() {
        let (a, b) = (random_key(), random_key());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
