//! Player skins and capes.
//!
//! A player's skin comes from the `textures` property of its profile (base64 JSON:
//! `textures.SKIN.url`, `SKIN.metadata.model == "slim"`, `textures.CAPE.url`) or from a player
//! skull's `Owner.Properties`. The page never fetches the texture host itself: this store downloads
//! each PNG once (allowed hosts only, no redirects, a timeout and a 64 KiB cap, a 64×64 / 64×32
//! PNG), keeps it in a small LRU by URL and serves it as `skins/<sha>.png` (content-addressed, so
//! the page may cache it for good). Downloads run on one background thread; [`SkinStore::ready`]
//! answers at once.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use sha2::Digest as _;

/// Textures of a profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Textures {
    pub skin: Option<String>,
    pub slim: bool,
    pub cape: Option<String>,
}

/// `textures.SKIN` / `CAPE` of a `textures` property value (base64 JSON).
#[must_use]
pub fn parse_textures(value: &str) -> Option<Textures> {
    if value.is_empty() || value.len() > 16384 {
        return None;
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(value.trim()))
        .ok()?;
    let j: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let t = j.get("textures")?.as_object()?;
    let url = |k: &str| {
        t.get(k)
            .and_then(|x| x.get("url"))
            .and_then(|u| u.as_str())
            .filter(|u| !u.is_empty())
            .map(str::to_string)
    };
    let slim = t
        .get("SKIN")
        .and_then(|s| s.get("metadata"))
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        == Some("slim");
    Some(Textures {
        skin: url("SKIN"),
        slim,
        cape: url("CAPE"),
    })
}

/// Vanilla's default skin for a profile without textures: Alex when `UUID.hashCode()` is odd
/// (the xor of the low bits of the four 32-bit words).
#[must_use]
pub fn slim_by_uuid(uuid: u128) -> bool {
    let words = [
        (uuid >> 96) as u32,
        (uuid >> 64) as u32,
        (uuid >> 32) as u32,
        uuid as u32,
    ];
    words.iter().fold(0, |b, w| b ^ (w & 1)) == 1
}

/// Hosts a texture URL may point at.
pub const TEXTURE_HOSTS: &[&str] = &["textures.minecraft.net"];

/// A texture URL we may download: http(s), an allowed host, a plain path, no query.
#[must_use]
pub fn texture_url_ok(raw: &str, hosts: &[String]) -> bool {
    if raw.len() > 300 {
        return false;
    }
    let rest = if let Some(r) = raw.strip_prefix("http://") {
        r
    } else if let Some(r) = raw.strip_prefix("https://") {
        r
    } else {
        return false;
    };
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => return false,
    };
    if host.contains('@') || !hosts.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        return false;
    }
    path.len() >= 2
        && path.len() <= 201
        && !path.contains("..")
        && path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'/'))
}

/// Width and height from a PNG's IHDR.
#[must_use]
pub fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    const SIG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    if b.len() < 33 || b[..8] != SIG || &b[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(b[16..20].try_into().ok()?),
        u32::from_be_bytes(b[20..24].try_into().ok()?),
    ))
}

/// Skins are 64×64 or 64×32; capes 64×32.
#[must_use]
pub fn skin_size_ok(size: Option<(u32, u32)>, cape: bool) -> bool {
    matches!(size, Some((64, 32))) || (!cape && matches!(size, Some((64, 64))))
}

/// Largest PNG accepted.
pub const MAX_SKIN_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct Inner {
    /// url → served path, LRU order (front = oldest).
    by_url: HashMap<String, String>,
    order: VecDeque<String>,
    files: HashMap<String, Arc<[u8]>>,
    pending: HashSet<String>,
    failed: HashMap<String, Instant>,
    queue: VecDeque<(String, bool)>,
    worker: bool,
}

/// Downloaded skins, shared by every viewer of the process.
pub struct SkinStore {
    inner: Mutex<Inner>,
    hosts: Vec<String>,
    max: usize,
    retry: Duration,
    #[cfg_attr(not(feature = "download"), allow(dead_code))]
    timeout: Duration,
}

impl Default for SkinStore {
    fn default() -> Self {
        SkinStore::new(TEXTURE_HOSTS.iter().map(|h| (*h).to_string()).collect())
    }
}

impl SkinStore {
    /// A store downloading from `hosts`.
    #[must_use]
    pub fn new(hosts: Vec<String>) -> Self {
        SkinStore {
            inner: Mutex::new(Inner::default()),
            hosts,
            max: 128,
            retry: Duration::from_secs(600),
            timeout: Duration::from_secs(5),
        }
    }

    /// The process-wide store.
    pub fn global() -> &'static Arc<SkinStore> {
        static STORE: std::sync::OnceLock<Arc<SkinStore>> = std::sync::OnceLock::new();
        STORE.get_or_init(|| Arc::new(SkinStore::default()))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Files kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().files.len()
    }

    /// No files kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The served path of a downloaded texture (marks it used), `None` when not (yet) here.
    pub fn ready(&self, url: &str) -> Option<String> {
        let mut g = self.lock();
        let path = g.by_url.get(url)?.clone();
        if let Some(i) = g.order.iter().position(|u| u == url) {
            let u = g.order.remove(i).unwrap_or_default();
            g.order.push_back(u);
        }
        Some(path)
    }

    /// Whether a download of `url` is queued or running.
    #[must_use]
    pub fn loading(&self, url: &str) -> bool {
        self.lock().pending.contains(url)
    }

    /// The PNG behind a served path (`skins/<32 hex>.png`).
    #[must_use]
    pub fn file(&self, path: &str) -> Option<Arc<[u8]>> {
        let name = path.strip_prefix("skins/")?.strip_suffix(".png")?;
        if name.len() != 32 || !name.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        self.lock().files.get(path).cloned()
    }

    /// Starts downloading `url` unless it is here, queued, invalid or failed recently.
    /// Returns whether a result may still come (poll [`SkinStore::ready`]).
    pub fn load(self: &Arc<Self>, url: &str, cape: bool) -> bool {
        if !texture_url_ok(url, &self.hosts) {
            return false;
        }
        let mut g = self.lock();
        if g.by_url.contains_key(url) || g.pending.contains(url) {
            return true;
        }
        if g.failed.get(url).is_some_and(|t| t.elapsed() < self.retry) {
            return false;
        }
        if !cfg!(feature = "download") {
            return false;
        }
        g.pending.insert(url.to_string());
        g.queue.push_back((url.to_string(), cape));
        if !g.worker {
            g.worker = true;
            let me = Arc::clone(self);
            let spawned = std::thread::Builder::new()
                .name("viewer-skins".into())
                .spawn(move || me.work());
            if spawned.is_err() {
                g.worker = false;
                g.pending.clear();
                g.queue.clear();
                return false;
            }
        }
        true
    }

    /// Stores a validated PNG under its content hash; returns the served path.
    pub fn put(&self, url: &str, png: Vec<u8>) -> String {
        let hash = sha2::Sha256::digest(&png);
        let hex: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
        let path = format!("skins/{hex}.png");
        let mut g = self.lock();
        if g.by_url.insert(url.to_string(), path.clone()).is_some() {
            g.order.retain(|u| u != url);
        }
        g.order.push_back(url.to_string());
        g.files.insert(path.clone(), png.into());
        while g.order.len() > self.max {
            let Some(old) = g.order.pop_front() else {
                break;
            };
            if let Some(p) = g.by_url.remove(&old)
                && !g.by_url.values().any(|v| *v == p)
            {
                g.files.remove(&p);
            }
        }
        path
    }

    fn work(self: Arc<Self>) {
        loop {
            let next = {
                let mut g = self.lock();
                match g.queue.pop_front() {
                    Some(n) => n,
                    None => {
                        g.worker = false;
                        return;
                    }
                }
            };
            let (url, cape) = next;
            let got = self.download(&url, cape);
            match got {
                Ok(png) => {
                    self.put(&url, png);
                    let mut g = self.lock();
                    g.failed.remove(&url);
                    g.pending.remove(&url);
                }
                Err(e) => {
                    log::debug!("[viewer] skin {url}: {e}");
                    let mut g = self.lock();
                    if g.failed.len() > 512 {
                        g.failed.clear();
                    }
                    g.failed.insert(url.clone(), Instant::now());
                    g.pending.remove(&url);
                }
            }
        }
    }

    #[cfg(feature = "download")]
    fn download(&self, url: &str, cape: bool) -> Result<Vec<u8>, String> {
        use std::io::Read as _;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .max_redirects(0)
            .build()
            .into();
        let res = agent.get(url).call().map_err(|e| e.to_string())?;
        let mut body = Vec::new();
        res.into_body()
            .into_reader()
            .take(MAX_SKIN_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        if body.len() > MAX_SKIN_BYTES {
            return Err("too big".into());
        }
        if !skin_size_ok(png_size(&body), cape) {
            return Err("not a skin png".into());
        }
        Ok(body)
    }

    #[cfg(not(feature = "download"))]
    fn download(&self, _url: &str, _cape: bool) -> Result<Vec<u8>, String> {
        Err("built without the download feature".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_are_parsed() {
        let json = r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/abc","metadata":{"model":"slim"}},"CAPE":{"url":"http://textures.minecraft.net/texture/def"}}}"#;
        let b64 = base64::engine::general_purpose::STANDARD.encode(json);
        let t = parse_textures(&b64).unwrap();
        assert!(t.slim);
        assert_eq!(
            t.skin.as_deref(),
            Some("http://textures.minecraft.net/texture/abc")
        );
        assert_eq!(
            t.cape.as_deref(),
            Some("http://textures.minecraft.net/texture/def")
        );
        assert!(parse_textures("not base64!").is_none());
    }

    #[test]
    fn urls_are_checked() {
        let hosts: Vec<String> = TEXTURE_HOSTS.iter().map(|h| (*h).to_string()).collect();
        assert!(texture_url_ok(
            "http://textures.minecraft.net/texture/abc123",
            &hosts
        ));
        assert!(!texture_url_ok("http://evil.example/texture/abc", &hosts));
        assert!(!texture_url_ok(
            "http://u@textures.minecraft.net/texture/abc",
            &hosts
        ));
        assert!(!texture_url_ok(
            "http://textures.minecraft.net/a?b=c",
            &hosts
        ));
        assert!(!texture_url_ok(
            "http://textures.minecraft.net/../x",
            &hosts
        ));
        assert!(!texture_url_ok("file:///etc/passwd", &hosts));
    }

    #[test]
    fn default_skin_parity() {
        // the four words' low bits: 1 ^ 0 ^ 0 ^ 0 = 1 → slim
        assert!(slim_by_uuid(1u128 << 96));
        assert!(!slim_by_uuid((1u128 << 96) | 1));
    }

    #[test]
    fn store_serves_by_content_hash() {
        let s = SkinStore::default();
        let p = s.put("http://textures.minecraft.net/texture/a", vec![1, 2, 3]);
        assert!(p.starts_with("skins/") && p.len() == "skins/".len() + 32 + 4);
        assert_eq!(
            s.ready("http://textures.minecraft.net/texture/a"),
            Some(p.clone())
        );
        assert_eq!(s.file(&p).as_deref(), Some(&[1u8, 2, 3][..]));
        assert!(s.file("skins/../x.png").is_none());
    }
}
