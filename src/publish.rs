use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::paths;

/// Tag scheme + presentation derived from the source ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseMeta {
    /// Tag name to publish under (e.g. "nightly" or "v1.0.0").
    pub tag: String,
    /// Human-readable title for the release.
    pub title: String,
    /// `true` for `nightly` and for any `v*-rc*` / `v*-beta*` tags.
    pub prerelease: bool,
    /// CHANGELOG.md section header to look up: `[Unreleased]` for nightly,
    /// `[X.Y.Z]` for a tagged release (with the `v` stripped).
    pub changelog_section: String,
}

/// Map a stored ref name + head sha to a release tag + title + prerelease flag.
pub fn derive_release_meta(ref_name: &str, head_sha: &str) -> ReleaseMeta {
    if ref_name == "refs/heads/master" {
        let short = short_sha(head_sha);
        return ReleaseMeta {
            tag: "nightly".to_string(),
            title: format!("Nightly build ({short})"),
            prerelease: true,
            changelog_section: "Unreleased".to_string(),
        };
    }
    if let Some(tag) = ref_name.strip_prefix("refs/tags/") {
        // v*-rc* / v*-beta* / v*-alpha* / v*-pre* are pre-releases.
        let prerelease = looks_like_prerelease_tag(tag);
        let section = tag.strip_prefix('v').unwrap_or(tag).to_string();
        return ReleaseMeta {
            tag: tag.to_string(),
            title: tag.to_string(),
            prerelease,
            changelog_section: section,
        };
    }
    // Unknown ref shape — caller shouldn't reach here, but fall back safely.
    let short = short_sha(head_sha);
    ReleaseMeta {
        tag: format!("dev-{short}"),
        title: format!("Dev build ({short})"),
        prerelease: true,
        changelog_section: "Unreleased".to_string(),
    }
}

fn looks_like_prerelease_tag(tag: &str) -> bool {
    let lower = tag.to_ascii_lowercase();
    lower.contains("-rc")
        || lower.contains("-beta")
        || lower.contains("-alpha")
        || lower.contains("-pre")
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(12).collect()
}

/// Collect all per-HOST signed release files we want to upload, plus the
/// detached GPG sidecars from the linux signer. Skips internal manifest
/// fragments (`SHA256SUMS.part`) — the combined SHA256SUMS is written by
/// the publisher itself.
pub fn collect_release_assets(workdir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(workdir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(entry.path())? {
            let f = f?;
            if !f.file_type()?.is_file() {
                continue;
            }
            let path = f.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if is_release_asset(name) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn is_release_asset(name: &str) -> bool {
    if name == "SHA256SUMS.part" {
        return false;
    }
    if !name.starts_with("navio-") {
        return false;
    }
    name.ends_with(".tar.gz")
        || name.ends_with(".tar.gz.asc")
        || name.ends_with(".zip")
        || name.ends_with(".zip.asc")
}

/// Compute SHA-256 for every asset and emit a deterministic SHA256SUMS
/// manifest (lines sorted by filename, lowercase hex, two spaces, basename
/// only).
pub fn write_combined_sha256sums(assets: &[PathBuf], dest: &Path) -> Result<()> {
    let mut lines: Vec<String> = Vec::with_capacity(assets.len());
    for path in assets {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let hex = paths::sha256_file(path)?;
        lines.push(format!("{hex}  {name}"));
    }
    lines.sort();
    let mut body = lines.join("\n");
    body.push('\n');
    std::fs::write(dest, body)?;
    Ok(())
}

/// Extract a single named section from a CHANGELOG-style markdown body.
/// Looks for a heading line beginning `## [<section>]` (matching whatever
/// comes between the brackets, prefix-style so `[1.0.0] - 2026-05-01` is
/// accepted), and returns the body up to (but not including) the next
/// `## [...]` heading. Returns None if no such heading.
pub fn parse_changelog_section(text: &str, section: &str) -> Option<String> {
    let want_prefix = format!("## [{section}]");
    let mut collecting = false;
    let mut out = String::new();
    for line in text.lines() {
        if line.starts_with("## [") {
            if collecting {
                break;
            }
            if line.starts_with(&want_prefix) {
                collecting = true;
                continue;
            }
        }
        if collecting {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !collecting {
        return None;
    }
    let trimmed = out.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_for_master() {
        let m = derive_release_meta("refs/heads/master", "0123456789abcdef0123");
        assert_eq!(m.tag, "nightly");
        assert!(m.prerelease);
        assert_eq!(m.changelog_section, "Unreleased");
        assert!(m.title.contains("0123456789ab"));
    }

    #[test]
    fn meta_for_tag_stable() {
        let m = derive_release_meta("refs/tags/v1.2.3", "deadbeefdeadbeefdead");
        assert_eq!(m.tag, "v1.2.3");
        assert!(!m.prerelease);
        assert_eq!(m.changelog_section, "1.2.3");
    }

    #[test]
    fn meta_for_tag_rc() {
        let m = derive_release_meta("refs/tags/v8.0.0-rc1", "abcd");
        assert_eq!(m.tag, "v8.0.0-rc1");
        assert!(m.prerelease);
        assert_eq!(m.changelog_section, "8.0.0-rc1");
    }

    #[test]
    fn meta_for_tag_beta() {
        let m = derive_release_meta("refs/tags/v0.1.0-beta2", "abcd");
        assert!(m.prerelease);
    }

    #[test]
    fn release_asset_filter() {
        assert!(is_release_asset("navio-abc-x86_64-linux-gnu.tar.gz"));
        assert!(is_release_asset("navio-abc-x86_64-linux-gnu.tar.gz.asc"));
        assert!(is_release_asset("navio-abc-x86_64-w64-mingw32.zip"));
        assert!(!is_release_asset("SHA256SUMS.part"));
        assert!(!is_release_asset("SHA256SUMS"));
        assert!(!is_release_asset("README.md"));
        assert!(!is_release_asset("something-else.tar.gz"));
    }

    #[test]
    fn changelog_section_extract() {
        let text = "# CHANGELOG\n\n\
            ## [Unreleased]\n\
            - foo\n\
            - bar\n\n\
            ## [1.2.3] - 2026-05-01\n\
            - baz\n";
        let unreleased = parse_changelog_section(text, "Unreleased").unwrap();
        assert!(unreleased.contains("- foo"));
        assert!(unreleased.contains("- bar"));
        assert!(!unreleased.contains("- baz"));

        let v123 = parse_changelog_section(text, "1.2.3").unwrap();
        assert!(v123.contains("- baz"));
        assert!(!v123.contains("- foo"));
    }

    #[test]
    fn changelog_section_missing() {
        let text = "## [1.0.0]\n- x\n";
        assert!(parse_changelog_section(text, "Unreleased").is_none());
    }
}
