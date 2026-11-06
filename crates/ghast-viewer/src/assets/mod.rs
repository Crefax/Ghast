//! The page's files: the web bundle (embedded at build time from `web/dist`), the block data and
//! the texture pack (built at run time from the vanilla client jar and cached on disk), all
//! served under content-hashed names (`v2/app.<hash>.js`, …) so pages cache them for good.

pub mod jar;
pub mod models;

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

use bytes::Bytes;
use sha1::Digest as _;

const APP: &[u8] = include_bytes!("../../web/dist/app.js");
const WORKER: &[u8] = include_bytes!("../../web/dist/worker.js");
const PAGE: &str = include_str!("../../web/dist/page.html");
const IMAGES: &str = include_str!("../../web/dist/images.json");
/// Bumped when the builder's output changes (invalidates cached builds).
const BUILDER: u32 = 1;

/// One served file.
#[derive(Debug, Clone)]
pub struct Asset {
    pub content_type: &'static str,
    pub body: Bytes,
    /// Gzipped body (for clients that accept it).
    pub gzip: Option<Bytes>,
}

/// The page and its files.
#[derive(Debug)]
pub struct Assets {
    files: HashMap<String, Asset>,
    page: String,
}

fn hash10(b: &[u8]) -> String {
    sha1::Sha1::digest(b)[..5]
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

fn gzip(b: &[u8]) -> Bytes {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let _ = e.write_all(b);
    Bytes::from(e.finish().unwrap_or_default())
}

/// The extra images the page needs (from the web bundle).
#[must_use]
pub fn images() -> Vec<String> {
    serde_json::from_str(IMAGES).unwrap_or_default()
}

impl Assets {
    /// Builds (or reads from `cache`) the data and pack from the client jar (`jar`, or the
    /// cached download).
    pub fn load(cache: &Path, jar: Option<&Path>) -> std::io::Result<Assets> {
        let t0 = std::time::Instant::now();
        let jar_path = jar::client_jar(cache, jar)?;
        let jar_sum = jar::sha1_file(&jar_path)?;
        let key: String = {
            let mut h = sha1::Sha1::new();
            h.update(jar_sum.as_bytes());
            h.update(IMAGES.as_bytes());
            h.update(env!("CARGO_PKG_VERSION").as_bytes());
            h.update(BUILDER.to_le_bytes());
            h.finalize()[..8]
                .iter()
                .map(|x| format!("{x:02x}"))
                .collect()
        };
        let dir = cache.join("built").join(&key);
        let (data, pack) = match (
            std::fs::read(dir.join("data.json")),
            std::fs::read(dir.join("pack.bin")),
        ) {
            (Ok(d), Ok(p)) => (d, p),
            _ => {
                let (d, p) = build(&jar_path)?;
                let tmp = cache
                    .join("built")
                    .join(format!("{key}.{}.tmp", std::process::id()));
                let _ = std::fs::remove_dir_all(&tmp);
                std::fs::create_dir_all(&tmp)?;
                std::fs::write(tmp.join("data.json"), &d)?;
                std::fs::write(tmp.join("pack.bin"), &p)?;
                // another bot process may have built it meanwhile: keep theirs
                if dir.exists() || std::fs::rename(&tmp, &dir).is_err() {
                    let _ = std::fs::remove_dir_all(&tmp);
                }
                // older builds (not builds under way: `<key>.<pid>.tmp`)
                if let Ok(rd) = std::fs::read_dir(cache.join("built")) {
                    for e in rd.flatten() {
                        let n = e.file_name().to_string_lossy().into_owned();
                        if n != key && !n.ends_with(".tmp") {
                            let _ = std::fs::remove_dir_all(e.path());
                        }
                    }
                }
                log::info!(
                    "[viewer] built assets in {} ms (data {} KB, pack {} KB)",
                    t0.elapsed().as_millis(),
                    d.len() >> 10,
                    p.len() >> 10
                );
                (d, p)
            }
        };
        Ok(Self::from_parts(data, pack))
    }

    fn from_parts(data: Vec<u8>, pack: Vec<u8>) -> Assets {
        let mut files = HashMap::new();
        let mut name =
            |stem: &str, ext: &str, body: Vec<u8>, ty: &'static str, gz: bool| -> String {
                let n = format!("{stem}.{}.{ext}", hash10(&body));
                let g = gz.then(|| gzip(&body));
                files.insert(
                    n.clone(),
                    Asset {
                        content_type: ty,
                        body: Bytes::from(body),
                        gzip: g,
                    },
                );
                n
            };
        let js = "application/javascript; charset=utf-8";
        let app = name("app", "js", APP.to_vec(), js, true);
        let worker = name("worker", "js", WORKER.to_vec(), js, true);
        let data = name(
            "data",
            "json",
            data,
            "application/json; charset=utf-8",
            true,
        );
        let pack = name("pack", "bin", pack, "application/octet-stream", false);
        let page = PAGE
            .replace("%APP%", &format!("v2/{app}"))
            .replace("%WORKER%", &format!("v2/{worker}"))
            .replace("%DATA%", &format!("v2/{data}"))
            .replace("%PACK%", &format!("v2/{pack}"));
        Assets { files, page }
    }

    /// A file under `v2/`.
    #[must_use]
    pub fn get(&self, file: &str) -> Option<&Asset> {
        self.files.get(file)
    }

    /// The page with the hashed file names.
    #[must_use]
    pub fn page(&self) -> &str {
        &self.page
    }
}

/// Reads the jar and builds `(data.json, pack.bin)`.
pub fn build(jar_path: &Path) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    const PREFIX: &str = "assets/minecraft/";
    let raw = jar::read_zip(jar_path, |n| {
        n.strip_prefix(PREFIX).is_some_and(|r| {
            r.starts_with("blockstates/") || r.starts_with("models/") || r.starts_with("textures/")
        })
    })?;
    let files: models::Files = raw
        .into_iter()
        .filter_map(|(k, v)| Some((k.strip_prefix(PREFIX)?.to_string(), v)))
        .collect();
    if !files.contains_key("textures/blocks/stone.png") {
        return Err(std::io::Error::other("not a client jar (no textures)"));
    }
    let (data, _) = models::build(&files, &images());
    let json = serde_json::to_vec(&data).map_err(std::io::Error::other)?;
    let pack = models::build_pack(&data, &files);
    Ok((json, pack))
}
