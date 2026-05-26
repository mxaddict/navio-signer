pub mod linux;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Append `.<ext>` to a path without replacing any existing extension.
/// `Path::with_extension` strips the trailing one, which is wrong for
/// double-extension files like `.tar.gz`.
pub fn append_extension(p: &Path, ext: &str) -> PathBuf {
    let mut os: OsString = p.as_os_str().to_owned();
    os.push(".");
    os.push(ext);
    PathBuf::from(os)
}

/// HOST classification by filename substring. Matches the guix-build naming:
/// `navio-<sha>-<HOST>.{tar.gz,zip}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Mingw,
    Darwin,
    Unknown,
}

impl Platform {
    pub fn from_filename(name: &str) -> Self {
        if name.contains("-linux-gnu") {
            Platform::Linux
        } else if name.contains("-w64-mingw32") {
            Platform::Mingw
        } else if name.contains("-apple-darwin") {
            Platform::Darwin
        } else {
            Platform::Unknown
        }
    }
}

/// Filename predicate for release archives produced by `guix-build`.
pub fn is_release_archive(name: &str) -> bool {
    name.starts_with("navio-") && (name.ends_with(".tar.gz") || name.ends_with(".zip"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_extension_preserves_compound() {
        let p = Path::new("/tmp/navio-abc.tar.gz");
        let out = append_extension(p, "asc");
        assert_eq!(out, Path::new("/tmp/navio-abc.tar.gz.asc"));
    }

    #[test]
    fn platform_from_name() {
        assert_eq!(
            Platform::from_filename("navio-abc-x86_64-linux-gnu.tar.gz"),
            Platform::Linux
        );
        assert_eq!(
            Platform::from_filename("navio-abc-arm-linux-gnueabihf.tar.gz"),
            Platform::Linux
        );
        assert_eq!(
            Platform::from_filename("navio-abc-x86_64-w64-mingw32.zip"),
            Platform::Mingw
        );
        assert_eq!(
            Platform::from_filename("navio-abc-arm64-apple-darwin.tar.gz"),
            Platform::Darwin
        );
        assert_eq!(
            Platform::from_filename("SHA256SUMS.part"),
            Platform::Unknown
        );
    }

    #[test]
    fn release_archive_predicate() {
        assert!(is_release_archive("navio-abc-x86_64-linux-gnu.tar.gz"));
        assert!(is_release_archive("navio-abc-x86_64-w64-mingw32.zip"));
        assert!(!is_release_archive("SHA256SUMS.part"));
        assert!(!is_release_archive("navio-abc-x86_64-linux-gnu.tar.gz.asc"));
        assert!(!is_release_archive("README.md"));
    }
}
