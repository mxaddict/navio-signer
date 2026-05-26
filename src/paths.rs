use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::db::Db;

/// Root data directory: `paths.data_dir` from config, else `$XDG_DATA_HOME/navio-signer`.
pub fn data_dir(cfg: &Config) -> Result<PathBuf> {
    Db::data_dir_for(cfg.paths.data_dir.as_deref())
}

/// Per-run working directory holding fetched + unzipped artifacts.
pub fn workdir_for(cfg: &Config, run_id: u64) -> Result<PathBuf> {
    Ok(data_dir(cfg)?.join("work").join(run_id.to_string()))
}

/// Extract a zip archive from in-memory bytes into `dest`. Refuses path
/// traversal (entries with `..` components or absolute paths).
pub fn extract_zip(zip_bytes: &[u8], dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).context("opening zip archive")?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).context("reading zip entry")?;
        let raw = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("zip entry has unsafe path: {}", entry.name()))?;
        if raw.is_absolute()
            || raw
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            bail!("zip entry has path traversal: {}", raw.display());
        }
        let out_path = dest.join(&raw);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .with_context(|| format!("creating {}", out_path.display()))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut buf)
            .context("reading zip entry body")?;
        std::fs::write(&out_path, &buf)
            .with_context(|| format!("writing {}", out_path.display()))?;
    }

    Ok(())
}

/// Stream a file through SHA-256 and return the hex digest (lowercase).
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher).with_context(|| format!("hashing {}", path.display()))?;
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        let tmp = std::env::temp_dir().join("navio-signer-sha-test");
        std::fs::write(&tmp, b"abc").unwrap();
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        let got = sha256_file(&tmp).unwrap();
        assert_eq!(
            got,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&tmp);
    }
}
