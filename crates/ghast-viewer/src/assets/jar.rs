//! The vanilla client jar: fetched from Mojang's version manifest (SHA-1 checked) into the
//! cache, and a minimal zip reader for the files the viewer needs.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha1::Digest as _;

/// The version whose assets the page draws.
pub const VERSION: &str = "1.8.9";
/// SHA-1 of the 1.8.9 client jar (from Mojang's version manifest).
pub const CLIENT_SHA1: &str = "3870888a6c3d349d3771a3e9d16c9bf5e076b908";
const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

fn other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// SHA-1 of a file, hex.
pub fn sha1_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The client jar: `configured` when given, else the cached copy, downloaded first if needed.
pub fn client_jar(cache: &Path, configured: Option<&Path>) -> std::io::Result<PathBuf> {
    if let Some(p) = configured {
        let sum = sha1_file(p)?;
        if sum != CLIENT_SHA1 {
            log::warn!(
                "[viewer] {}: SHA-1 {sum} is not the {VERSION} client jar's; using it anyway",
                p.display()
            );
        }
        return Ok(p.to_path_buf());
    }
    let path = cache.join(format!("client-{VERSION}.jar"));
    if path.exists() && sha1_file(&path)? == CLIENT_SHA1 {
        return Ok(path);
    }
    download(&path)?;
    Ok(path)
}

#[cfg(feature = "download")]
fn download(path: &Path) -> std::io::Result<()> {
    let get_json = |url: &str| -> std::io::Result<serde_json::Value> {
        let body = ureq::get(url)
            .call()
            .map_err(other)?
            .into_body()
            .read_to_vec()
            .map_err(other)?;
        serde_json::from_slice(&body).map_err(other)
    };
    log::info!("[viewer] downloading the {VERSION} client jar from Mojang (textures and models)");
    let manifest = get_json(MANIFEST)?;
    let url = manifest["versions"]
        .as_array()
        .and_then(|v| v.iter().find(|x| x["id"] == VERSION))
        .and_then(|v| v["url"].as_str())
        .ok_or_else(|| other("version not in the manifest"))?
        .to_string();
    let version = get_json(&url)?;
    let client = &version["downloads"]["client"];
    let jar_url = client["url"]
        .as_str()
        .ok_or_else(|| other("no client download"))?;
    if client["sha1"].as_str() != Some(CLIENT_SHA1) {
        return Err(other("the manifest's client jar has an unexpected SHA-1"));
    }
    let body = ureq::get(jar_url)
        .call()
        .map_err(other)?
        .into_body()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .map_err(other)?;
    let sum: String = sha1::Sha1::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if sum != CLIENT_SHA1 {
        return Err(other(format!("downloaded jar has SHA-1 {sum}")));
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension(format!("jar.{}.part", std::process::id()));
    std::fs::write(&tmp, &body)?;
    std::fs::rename(&tmp, path)
}

#[cfg(not(feature = "download"))]
fn download(path: &Path) -> std::io::Result<()> {
    Err(other(format!(
        "no client jar at {} (built without the download feature: set ViewerConfig::client_jar)",
        path.display()
    )))
}

fn u16_at(b: &[u8], at: usize) -> std::io::Result<usize> {
    b.get(at..at + 2)
        .map(|s| usize::from(u16::from_le_bytes([s[0], s[1]])))
        .ok_or_else(|| other("zip: truncated"))
}
fn u32_at(b: &[u8], at: usize) -> std::io::Result<usize> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
        .ok_or_else(|| other("zip: truncated"))
}

/// Reads the entries of a zip whose names `want` accepts (stored or deflated; no zip64).
pub fn read_zip(
    path: &Path,
    want: impl Fn(&str) -> bool,
) -> std::io::Result<HashMap<String, Vec<u8>>> {
    let data = std::fs::read(path)?;
    // end of central directory: the last signature within the final 64 KiB + 22 bytes
    let from = data.len().saturating_sub(65_557);
    let eocd = (from..data.len().saturating_sub(21))
        .rev()
        .find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| other("zip: no end of central directory"))?;
    let count = u16_at(&data, eocd + 10)?;
    let mut at = u32_at(&data, eocd + 16)?;
    let mut out = HashMap::new();
    for _ in 0..count {
        if data.get(at..at + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            return Err(other("zip: bad central directory"));
        }
        let method = u16_at(&data, at + 10)?;
        let csize = u32_at(&data, at + 20)?;
        let usize_ = u32_at(&data, at + 24)?;
        let (n, e, c) = (
            u16_at(&data, at + 28)?,
            u16_at(&data, at + 30)?,
            u16_at(&data, at + 32)?,
        );
        let local = u32_at(&data, at + 42)?;
        let name = String::from_utf8_lossy(
            data.get(at + 46..at + 46 + n)
                .ok_or_else(|| other("zip: truncated"))?,
        )
        .into_owned();
        at += 46 + n + e + c;
        if name.ends_with('/') || !want(&name) {
            continue;
        }
        let ln = u16_at(&data, local + 26)?;
        let le = u16_at(&data, local + 28)?;
        let start = local + 30 + ln + le;
        let raw = data
            .get(start..start + csize)
            .ok_or_else(|| other("zip: truncated entry"))?;
        let body = match method {
            0 => raw.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw).read_to_end(&mut v)?;
                v
            }
            m => return Err(other(format!("zip: method {m} in {name}"))),
        };
        out.insert(name, body);
    }
    Ok(out)
}
