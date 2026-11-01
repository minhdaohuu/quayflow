//! Static canary (FR-24): the client binary still carries the lock and login names the
//! adapter relies on. A missing marker does not prove the behavior changed, but the
//! adapter was built against them, so in-place switching stops for that version.

use std::io::Read;
use std::path::{Path, PathBuf};

pub const MARKERS: &[(&str, &str)] = &[
    ("OAuth-refresh lock (.oauth_refresh.lock)", ".oauth_refresh.lock"),
    (
        "legacy credential lock handling",
        "tengu_oauth_refresh_legacy_lock_contended",
    ),
    ("credential file (.credentials.json)", ".credentials.json"),
    ("login entry (claudeAiOauth)", "claudeAiOauth"),
    ("account entry (oauthAccount)", "oauthAccount"),
];

/// Where the native installer keeps a version's binary.
pub fn version_binary(version: &str) -> Option<PathBuf> {
    if version.is_empty()
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    let p = crate::env::home_dir()
        .join(".local")
        .join("share")
        .join("claude")
        .join("versions")
        .join(version);
    p.is_file().then_some(p)
}

/// Stream-search a file for every marker. Returns (label, found) per marker.
pub fn scan(path: &Path) -> std::io::Result<Vec<(String, bool)>> {
    let mut f = std::fs::File::open(path)?;
    let longest = MARKERS.iter().map(|(_, m)| m.len()).max().unwrap_or(0);
    let finders: Vec<memchr::memmem::Finder> = MARKERS
        .iter()
        .map(|(_, m)| memchr::memmem::Finder::new(m.as_bytes()))
        .collect();
    let mut found = vec![false; MARKERS.len()];
    let mut buf = vec![0u8; 8 << 20];
    let mut carry: Vec<u8> = vec![];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let mut window = std::mem::take(&mut carry);
        window.extend_from_slice(&buf[..n]);
        for (i, finder) in finders.iter().enumerate() {
            if !found[i] && finder.find(&window).is_some() {
                found[i] = true;
            }
        }
        if found.iter().all(|x| *x) {
            break;
        }
        let keep = window.len().min(longest);
        carry = window[window.len() - keep..].to_vec();
    }
    Ok(MARKERS
        .iter()
        .zip(found)
        .map(|((label, _), ok)| (label.to_string(), ok))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_markers_across_chunk_boundaries() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("bin");
        let mut data = vec![b'x'; (8 << 20) - 5];
        data.extend_from_slice(
            b"claudeAiOauth .oauth_refresh.lock tengu_oauth_refresh_legacy_lock_contended .credentials.json",
        );
        std::fs::write(&p, &data).unwrap();
        let r = scan(&p).unwrap();
        assert!(r.iter().filter(|(l, _)| !l.contains("oauthAccount")).all(|(_, ok)| *ok));
        assert!(!r.iter().find(|(l, _)| l.contains("oauthAccount")).unwrap().1);
    }

    #[test]
    fn real_binary_if_present() {
        if let Some(p) = version_binary("2.1.316") {
            assert!(scan(&p).unwrap().iter().all(|(_, ok)| *ok));
        }
    }
}
