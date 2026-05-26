use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tar::{Archive, Builder, EntryType, Header};
use tracing::{debug, info};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::config::MacosSigningConfig;
use crate::sign::append_extension;

/// SOURCE_DATE_EPOCH used for repacking; matches the bitcoin/navio guix
/// convention of pinning every release tarball's mtime to 0 (Unix epoch),
/// which is what `tar` emits by default when fed a 0 mtime header. The
/// originals come out of guix-build with this same value, so for entries
/// we don't modify the new archive is bit-for-bit identical.
const DETERMINISTIC_MTIME: u64 = 0;

/// Codesign every Mach-O binary inside `tarball` (a `.tar.gz` from guix
/// build for an `*-apple-darwin` HOST), notarize the bundle, and repack
/// the tarball deterministically over the original. The tarball is
/// modified in-place via an atomic rename.
///
/// No stapling is attempted: bare Mach-O binaries cannot carry a stapled
/// notarization ticket (Apple supports stapling only on .app / .pkg /
/// .dmg). Gatekeeper performs an online ticket lookup the first time the
/// binary is executed; if offline, the user sees a one-time spinner.
pub fn sign_tarball(tarball: &Path, cfg: &MacosSigningConfig) -> Result<()> {
    let raw =
        std::fs::read(tarball).with_context(|| format!("reading tarball {}", tarball.display()))?;
    let mut entries =
        read_tar_gz(&raw).with_context(|| format!("parsing tarball {}", tarball.display()))?;

    // Stage Mach-O entries on disk so codesign can rewrite them in place.
    let staging = tempfile::tempdir().context("creating staging dir for codesign")?;
    let mut signed_paths: Vec<PathBuf> = Vec::new();
    let mut signed_count = 0u32;
    for entry in entries.iter_mut() {
        if entry.entry_type != EntryType::Regular {
            continue;
        }
        if !is_macho(&entry.content) {
            continue;
        }
        let staged = staging.path().join(&entry.path);
        if let Some(parent) = staged.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&staged, &entry.content)
            .with_context(|| format!("staging {}", staged.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(entry.mode))?;
        }
        codesign(&staged, &cfg.identity).with_context(|| format!("codesigning {}", entry.path))?;
        entry.content = std::fs::read(&staged)
            .with_context(|| format!("reading signed {}", staged.display()))?;
        signed_paths.push(staged);
        signed_count += 1;
    }

    if signed_count == 0 {
        bail!(
            "no Mach-O binaries found in {} — nothing to sign",
            tarball.display()
        );
    }

    // Bundle all signed Mach-O binaries into a zip for notarytool.
    let notarize_zip = staging.path().join("notarize.zip");
    write_notarize_zip(&notarize_zip, &signed_paths, staging.path())
        .with_context(|| format!("building notarization zip at {}", notarize_zip.display()))?;
    notarytool_submit(&notarize_zip, &cfg.keychain_profile).context("notarytool submit failed")?;

    // Repack tar.gz over the original atomically.
    let tmp_path = append_extension(tarball, "tmp");
    write_tar_gz_deterministic(&tmp_path, &entries)
        .with_context(|| format!("writing {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, tarball)
        .with_context(|| format!("renaming {} -> {}", tmp_path.display(), tarball.display()))?;

    info!(
        tarball = %tarball.display(),
        signed_count,
        "darwin tarball resigned + notarized"
    );
    Ok(())
}

/// In-memory tar entry for round-tripping the archive. We don't carry
/// hardlinks / symlinks / device nodes — guix-built navio tarballs use
/// only regular files and directories.
#[derive(Debug, Clone)]
struct TarEntry {
    path: String,
    mode: u32,
    entry_type: EntryType,
    content: Vec<u8>,
}

fn read_tar_gz(bytes: &[u8]) -> Result<Vec<TarEntry>> {
    let gz = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(gz);
    let mut out = Vec::new();
    for entry in archive.entries().context("iterating tar entries")? {
        let mut e = entry.context("reading tar entry")?;
        let header = e.header().clone();
        let path = e
            .path()
            .context("decoding entry path")?
            .to_string_lossy()
            .into_owned();
        let mode = header.mode().unwrap_or(0o644);
        let entry_type = header.entry_type();
        let mut content = Vec::with_capacity(e.size() as usize);
        if entry_type == EntryType::Regular {
            e.read_to_end(&mut content)
                .with_context(|| format!("reading body of {path}"))?;
        }
        out.push(TarEntry {
            path,
            mode,
            entry_type,
            content,
        });
    }
    Ok(out)
}

fn write_tar_gz_deterministic(out_path: &Path, entries: &[TarEntry]) -> Result<()> {
    // Sort by path for stable ordering. Use a BTreeMap so duplicate paths
    // (which would be a corrupt tar) collapse instead of being silently
    // re-ordered.
    let mut sorted: BTreeMap<&str, &TarEntry> = BTreeMap::new();
    for entry in entries {
        sorted.insert(entry.path.as_str(), entry);
    }

    let file =
        File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
    // gzip header: clear mtime field (set to 0) by writing with Compression
    // level and not passing OS-specific fields. flate2's GzEncoder writes a
    // minimal header with mtime=0 by default.
    let gz = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(gz);
    builder.mode(tar::HeaderMode::Deterministic);

    for entry in sorted.values() {
        let mut header = Header::new_gnu();
        header
            .set_path(&entry.path)
            .with_context(|| format!("setting path {}", entry.path))?;
        header.set_mode(entry.mode);
        header.set_mtime(DETERMINISTIC_MTIME);
        header.set_uid(0);
        header.set_gid(0);
        header.set_entry_type(entry.entry_type);
        header.set_size(entry.content.len() as u64);
        header.set_cksum();

        builder
            .append(&header, entry.content.as_slice())
            .with_context(|| format!("appending {}", entry.path))?;
    }
    let gz = builder.into_inner().context("finalising tar")?;
    gz.finish().context("finalising gzip stream")?;
    Ok(())
}

fn write_notarize_zip(out_path: &Path, files: &[PathBuf], base_dir: &Path) -> Result<()> {
    let file =
        File::create(out_path).with_context(|| format!("creating {}", out_path.display()))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for path in files {
        let rel = path
            .strip_prefix(base_dir)
            .with_context(|| format!("path not under base: {}", path.display()))?;
        let name = rel.to_string_lossy().replace('\\', "/");
        writer
            .start_file(name.as_str(), options)
            .with_context(|| format!("starting zip entry {name}"))?;
        let body = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        writer
            .write_all(&body)
            .with_context(|| format!("writing zip entry {name}"))?;
    }
    writer.finish().context("finalising notarization zip")?;
    Ok(())
}

/// Mach-O / fat magic byte sniff. We don't try to validate beyond the
/// magic since `codesign` will rerefuse anything malformed.
pub fn is_macho(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let m = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    matches!(
        m,
        0xFEED_FACE        // 32-bit BE
            | 0xFEED_FACF  // 64-bit BE
            | 0xCEFA_EDFE  // 32-bit LE
            | 0xCFFA_EDFE  // 64-bit LE
            | 0xCAFE_BABE  // fat BE
            | 0xBEBA_FECA // fat LE
    )
}

fn codesign(path: &Path, identity: &str) -> Result<()> {
    debug!(file = %path.display(), "codesign --force --options runtime --timestamp");
    let status = Command::new("codesign")
        .arg("--sign")
        .arg(identity)
        .arg("--options")
        .arg("runtime")
        .arg("--timestamp")
        .arg("--force")
        .arg(path)
        .status()
        .with_context(|| format!("running codesign on {}", path.display()))?;
    if !status.success() {
        bail!(
            "codesign exited {} for {}",
            status.code().unwrap_or(-1),
            path.display()
        );
    }
    Ok(())
}

/// Submit the bundle to Apple's notary service and block until the
/// service returns a verdict (`--wait`). We don't staple — stapling is
/// not possible for bare Mach-O binaries, only for `.app` / `.pkg` /
/// `.dmg`. Gatekeeper resolves the notarization ticket online when the
/// binary is executed.
fn notarytool_submit(zip: &Path, keychain_profile: &str) -> Result<()> {
    info!(zip = %zip.display(), profile = keychain_profile, "xcrun notarytool submit --wait");
    let status = Command::new("xcrun")
        .arg("notarytool")
        .arg("submit")
        .arg(zip)
        .arg("--keychain-profile")
        .arg(keychain_profile)
        .arg("--wait")
        .status()
        .with_context(|| format!("running notarytool on {}", zip.display()))?;
    if !status.success() {
        bail!(
            "notarytool submit exited {} (apple rejection? check `xcrun notarytool log` with the submission ID)",
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macho_magic_detect() {
        assert!(is_macho(&[0xFE, 0xED, 0xFA, 0xCE, 0xAA]));
        assert!(is_macho(&[0xCF, 0xFA, 0xED, 0xFE]));
        assert!(is_macho(&[0xCA, 0xFE, 0xBA, 0xBE]));
        assert!(!is_macho(&[0x7F, b'E', b'L', b'F']));
        assert!(!is_macho(&[0x4D, 0x5A])); // PE
        assert!(!is_macho(&[]));
        assert!(!is_macho(&[0x1F, 0x8B])); // gzip
    }

    #[test]
    fn tar_round_trip_deterministic() {
        let entries = vec![
            TarEntry {
                path: "bin/naviod".into(),
                mode: 0o755,
                entry_type: EntryType::Regular,
                content: b"binary data".to_vec(),
            },
            TarEntry {
                path: "README.md".into(),
                mode: 0o644,
                entry_type: EntryType::Regular,
                content: b"readme".to_vec(),
            },
        ];

        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("a.tar.gz");
        let path_b = dir.path().join("b.tar.gz");
        write_tar_gz_deterministic(&path_a, &entries).unwrap();
        write_tar_gz_deterministic(&path_b, &entries).unwrap();

        let a = std::fs::read(&path_a).unwrap();
        let b = std::fs::read(&path_b).unwrap();
        assert_eq!(a, b, "deterministic tar.gz must be byte-identical");

        let roundtripped = read_tar_gz(&a).unwrap();
        assert_eq!(roundtripped.len(), 2);
        // BTreeMap sort orders README.md before bin/naviod
        assert_eq!(roundtripped[0].path, "README.md");
        assert_eq!(roundtripped[1].path, "bin/naviod");
        assert_eq!(roundtripped[1].mode, 0o755);
    }
}
