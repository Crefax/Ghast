//! Idempotent download of the pinned minecraft-data npm package into the git-ignored cache.
//!
//! Layout (shared with `codegen/protocol`, which uses the same pin):
//!
//! ```text
//! .cache/minecraft-data/dl/minecraft-data-<ver>.tgz      verified npm tarball
//! .cache/minecraft-data/<ver>/package/minecraft-data/data the data tree (dataPaths.json, pc/...)
//! ```
//!
//! Concurrency: every step writes to a process-unique temporary name and then `rename`s it into
//! place, so two generators running at once never observe a half-written file. If the target
//! already exists when we rename, the other process won and we use its (identical) result.
//!
//! Uses the system `curl`, `tar` and `sha512sum` (present on Linux, WSL and Git Bash) to avoid
//! pulling an HTTP/TLS stack into the build.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Pinned minecraft-data version.
pub const VERSION: &str = "3.119.0";
/// npm `dist.integrity` (SHA-512, base64) of the pinned tarball.
const INTEGRITY_SHA512_B64: &str =
    "RElo0o2RooEzpeQO+QEHzTmHVTgOdbbomGQYBmWIaU1pupEmAItQb+LpJyL2EDXmpKicOU4swtBVzwIWae5l5g==";

fn run(cmd: &mut Command) -> Result<Vec<u8>, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run {cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{cmd:?} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn verify(tgz: &Path) -> Result<(), String> {
    let out = run(Command::new("sha512sum").arg(tgz))?;
    let hex = String::from_utf8_lossy(&out);
    let hex = hex.split_whitespace().next().unwrap_or_default();
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0))
        .collect();
    let got = base64(&bytes);
    if got != INTEGRITY_SHA512_B64 {
        return Err(format!(
            "{} has sha512 {got}, expected {INTEGRITY_SHA512_B64}",
            tgz.display()
        ));
    }
    Ok(())
}

/// Ensures the data tree exists under `cache` and returns the `.../minecraft-data/data` directory.
pub fn ensure(cache: &Path, offline: bool) -> Result<PathBuf, String> {
    let data = cache.join(VERSION).join("package/minecraft-data/data");
    if data.join("dataPaths.json").is_file() {
        return Ok(data);
    }
    if offline {
        return Err(format!("{} missing and --offline given", data.display()));
    }
    let pid = std::process::id();
    let dl = cache.join("dl");
    std::fs::create_dir_all(&dl).map_err(|e| e.to_string())?;
    let tgz = dl.join(format!("minecraft-data-{VERSION}.tgz"));
    if !tgz.is_file() || verify(&tgz).is_err() {
        let tmp = dl.join(format!("minecraft-data-{VERSION}.tgz.tmp.{pid}"));
        let url =
            format!("https://registry.npmjs.org/minecraft-data/-/minecraft-data-{VERSION}.tgz");
        eprintln!("downloading {url}");
        run(Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&tmp)
            .arg(&url))?;
        verify(&tmp)?;
        std::fs::rename(&tmp, &tgz).map_err(|e| e.to_string())?;
    }
    verify(&tgz)?;
    let tmp_dir = cache.join(format!("{VERSION}.tmp.{pid}"));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    run(Command::new("tar")
        .arg("-xzf")
        .arg(&tgz)
        .arg("-C")
        .arg(&tmp_dir))?;
    match std::fs::rename(&tmp_dir, cache.join(VERSION)) {
        Ok(()) => {}
        // Someone else extracted it first; theirs is identical.
        Err(_) if data.join("dataPaths.json").is_file() => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
        }
        Err(e) => {
            return Err(format!(
                "rename into {}: {e}",
                cache.join(VERSION).display()
            ));
        }
    }
    if !data.join("dataPaths.json").is_file() {
        return Err(format!(
            "{} has no dataPaths.json after extraction",
            data.display()
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(super::base64(b"foob"), "Zm9vYg==");
    }
}
