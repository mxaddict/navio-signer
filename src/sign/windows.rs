use anyhow::{Context, Result, bail};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::process::Command;
use tracing::{debug, info};
use zip::{ZipArchive, ZipWriter};

use crate::config::WindowsSigningConfig;
use crate::sign::append_extension;

/// Sign every `.exe` and `.dll` inside the mingw release zip using
/// Authenticode (osslsigncode) and repack the zip deterministically.
///
/// The repack preserves each surviving entry's original options (compression,
/// permissions, mtime) via `ZipFile::options()` and writes entries sorted by
/// name. Signed binaries replace their originals in-place.
///
/// Atomic-rename via a `.tmp` sidecar so a partial write cannot corrupt the
/// original artifact.
pub fn sign_zip(zip_path: &Path, cfg: &WindowsSigningConfig) -> Result<()> {
    let original =
        std::fs::read(zip_path).with_context(|| format!("reading zip {}", zip_path.display()))?;

    let mut entries =
        read_entries(&original).with_context(|| format!("parsing zip {}", zip_path.display()))?;

    let mut signed = 0u32;
    for entry in entries.iter_mut() {
        if !is_signable_pe(&entry.name) {
            continue;
        }
        info!(file = entry.name.as_str(), "osslsigncode sign");
        let new_content = sign_blob(&entry.content, &entry.name, cfg)
            .with_context(|| format!("signing {}", entry.name))?;
        entry.content = new_content;
        signed += 1;
    }

    if signed == 0 {
        bail!(
            "no .exe / .dll entries found in {} — nothing to sign",
            zip_path.display()
        );
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let tmp_path = append_extension(zip_path, "tmp");
    {
        let tmp_file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating {}", tmp_path.display()))?;
        let mut writer = ZipWriter::new(tmp_file);
        for entry in entries {
            writer
                .start_file(&entry.name, entry.options)
                .with_context(|| format!("writing entry {}", entry.name))?;
            writer
                .write_all(&entry.content)
                .with_context(|| format!("writing body of {}", entry.name))?;
        }
        writer.finish().context("finalising zip")?;
    }

    std::fs::rename(&tmp_path, zip_path)
        .with_context(|| format!("renaming {} -> {}", tmp_path.display(), zip_path.display()))?;
    info!(
        zip = %zip_path.display(),
        signed,
        "windows zip resigned"
    );
    Ok(())
}

struct Entry {
    name: String,
    content: Vec<u8>,
    options: zip::write::SimpleFileOptions,
}

fn read_entries(bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).context("opening zip")?;
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).context("reading zip entry")?;
        // We don't carry directory entries forward — the zip crate emits
        // implicit directory headers when needed, and the guix-built mingw
        // archive doesn't use any.
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let options = entry.options();
        let mut content = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut content)
            .with_context(|| format!("reading entry {name}"))?;
        out.push(Entry {
            name,
            content,
            options,
        });
    }
    Ok(out)
}

fn is_signable_pe(name: &str) -> bool {
    // Match on the basename's extension so a hypothetical
    // `subdir/foo.exe` is still recognised.
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".exe") || lower.ends_with(".dll")
}

/// Sign a single PE binary via osslsigncode. Writes the input to a temp
/// file (osslsigncode requires file paths), invokes `osslsigncode sign`
/// with the configured PKCS#12 cert + RFC3161 TSA, then `osslsigncode
/// verify` on the output. Returns the signed bytes.
fn sign_blob(content: &[u8], display_name: &str, cfg: &WindowsSigningConfig) -> Result<Vec<u8>> {
    let dir = tempfile::tempdir().context("creating temp dir for osslsigncode")?;
    let in_path = dir.path().join("input");
    let out_path = dir.path().join("output");

    std::fs::write(&in_path, content)
        .with_context(|| format!("writing {} to temp", display_name))?;

    let status = Command::new("osslsigncode")
        .arg("sign")
        .arg("-pkcs12")
        .arg(&cfg.pkcs12_path)
        .arg("-pass")
        .arg(&cfg.pkcs12_password)
        .arg("-t")
        .arg(&cfg.timestamp_url)
        .arg("-in")
        .arg(&in_path)
        .arg("-out")
        .arg(&out_path)
        .status()
        .context("running osslsigncode sign")?;
    if !status.success() {
        bail!(
            "osslsigncode sign exited {} for {display_name}",
            status.code().unwrap_or(-1)
        );
    }

    debug!(file = display_name, "osslsigncode verify");
    let verify = Command::new("osslsigncode")
        .arg("verify")
        .arg(&out_path)
        .status()
        .context("running osslsigncode verify")?;
    if !verify.success() {
        bail!(
            "osslsigncode verify exited {} for {display_name}",
            verify.code().unwrap_or(-1)
        );
    }

    let signed = std::fs::read(&out_path)
        .with_context(|| format!("reading signed output for {display_name}"))?;
    Ok(signed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signable_extensions() {
        assert!(is_signable_pe("naviod.exe"));
        assert!(is_signable_pe("NAVIO.DLL"));
        assert!(is_signable_pe("subdir/foo.exe"));
        assert!(!is_signable_pe("README"));
        assert!(!is_signable_pe("foo.exe.bak"));
    }
}
