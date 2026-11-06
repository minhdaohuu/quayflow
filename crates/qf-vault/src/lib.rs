//! Credential vault (FR-21).
//!
//! Each registered account's login (`claudeAiOauth` + `oauthAccount`) is sealed with
//! Windows DPAPI at current-user scope plus Quayflow entropy, with UI prompts forbidden,
//! and stored as base64 text in `vault\<account-id>.dpapi` (the same format the PoC wrote).
//! There is no plaintext fallback: if sealing fails, nothing is stored.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const ENTROPY: &[u8] = b"quayflow-vault-v1";

/// A Claude login: the two account-specific entries Quayflow swaps.
/// Debug output never shows the contents.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Login {
    /// `claudeAiOauth` from `.credentials.json`.
    pub oauth: serde_json::Value,
    /// `oauthAccount` from `.claude.json`.
    pub account: serde_json::Value,
}

impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Login {{ account: #{} }}", self.account_hash().unwrap_or_default())
    }
}

impl Login {
    pub fn account_uuid(&self) -> Option<&str> {
        self.account.get("accountUuid").and_then(|v| v.as_str())
    }
    pub fn org_uuid(&self) -> Option<&str> {
        self.account.get("organizationUuid").and_then(|v| v.as_str())
    }
    pub fn org_name(&self) -> Option<String> {
        self.account
            .get("organizationName")
            .and_then(|v| v.as_str())
            .map(String::from)
    }
    pub fn account_hash(&self) -> Option<String> {
        self.account_uuid().map(qf_model::identity_hash)
    }
    pub fn access_token(&self) -> Option<&str> {
        self.oauth.get("accessToken").and_then(|v| v.as_str())
    }
    pub fn refresh_token(&self) -> Option<&str> {
        self.oauth.get("refreshToken").and_then(|v| v.as_str())
    }
    pub fn expires_at(&self) -> Option<i64> {
        self.oauth.get("expiresAt").and_then(|v| v.as_i64())
    }
    pub fn subscription(&self) -> Option<&str> {
        self.oauth.get("subscriptionType").and_then(|v| v.as_str())
    }
    pub fn rate_limit_tier(&self) -> Option<&str> {
        self.oauth.get("rateLimitTier").and_then(|v| v.as_str())
    }
    /// Same identity and same tokens.
    pub fn same_tokens(&self, other: &Login) -> bool {
        self.account_uuid() == other.account_uuid()
            && self.access_token() == other.access_token()
            && self.refresh_token() == other.refresh_token()
    }
    pub fn kind(&self) -> qf_model::AccountKind {
        match self.subscription().map(|s| s.to_ascii_lowercase()) {
            Some(s) if s == "team" || s == "enterprise" => qf_model::AccountKind::Organization,
            _ => qf_model::AccountKind::Personal,
        }
    }
}

#[derive(Debug)]
pub enum VaultError {
    /// DPAPI refused (wrong user, corrupted blob, profile problem). No fallback.
    Protect(String),
    Io(std::io::Error),
    Format(String),
    NotFound,
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::Protect(m) => write!(f, "protected storage failed: {m}"),
            VaultError::Io(e) => write!(f, "vault file error: {e}"),
            VaultError::Format(m) => write!(f, "vault entry unreadable: {m}"),
            VaultError::NotFound => write!(f, "no vault entry"),
        }
    }
}

impl std::error::Error for VaultError {}

impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            VaultError::NotFound
        } else {
            VaultError::Io(e)
        }
    }
}

/// DPAPI seal/unseal. Behind a trait so tests can run the vault logic deterministically.
pub trait Protector: Send + Sync {
    fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, VaultError>;
    fn unprotect(&self, sealed: &[u8]) -> Result<Vec<u8>, VaultError>;
}

pub struct Dpapi;

#[cfg(windows)]
mod dpapi {
    use super::*;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    unsafe fn take(out: &CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        v
    }

    impl Protector for Dpapi {
        fn protect(&self, plain: &[u8]) -> Result<Vec<u8>, VaultError> {
            let input = blob(plain);
            let entropy = blob(ENTROPY);
            let mut out = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            // SAFETY: input/entropy point to live slices for the call; out is freed by `take`.
            let ok = unsafe {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            };
            if ok == 0 {
                return Err(VaultError::Protect(std::io::Error::last_os_error().to_string()));
            }
            Ok(unsafe { take(&out) })
        }

        fn unprotect(&self, sealed: &[u8]) -> Result<Vec<u8>, VaultError> {
            let input = blob(sealed);
            let entropy = blob(ENTROPY);
            let mut out = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            // SAFETY: as above.
            let ok = unsafe {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    &entropy,
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            };
            if ok == 0 {
                return Err(VaultError::Protect(std::io::Error::last_os_error().to_string()));
            }
            Ok(unsafe { take(&out) })
        }
    }
}

#[cfg(not(windows))]
impl Protector for Dpapi {
    fn protect(&self, _: &[u8]) -> Result<Vec<u8>, VaultError> {
        Err(VaultError::Protect("DPAPI is only available on Windows".into()))
    }
    fn unprotect(&self, _: &[u8]) -> Result<Vec<u8>, VaultError> {
        Err(VaultError::Protect("DPAPI is only available on Windows".into()))
    }
}

/// Write a file by temp + rename, retrying sharing violations.
///
/// The temp name is unique per call (process id + counter), so concurrent writers of the
/// same target never share a temp file, and the temp file (which may hold tokens) is
/// removed on every error path.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.quayflow-{}-{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let mut attempt = 0;
    loop {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 10 && is_sharing_violation(&e) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(100 * attempt));
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
}

pub fn is_sharing_violation(e: &std::io::Error) -> bool {
    // ERROR_ACCESS_DENIED 5, ERROR_SHARING_VIOLATION 32, ERROR_LOCK_VIOLATION 33
    matches!(e.raw_os_error(), Some(5) | Some(32) | Some(33)) || e.kind() == std::io::ErrorKind::PermissionDenied
}

pub struct Vault {
    dir: PathBuf,
    protector: Box<dyn Protector>,
}

impl Vault {
    pub fn new(dir: PathBuf) -> Self {
        Vault {
            dir,
            protector: Box::new(Dpapi),
        }
    }

    pub fn with_protector(dir: PathBuf, protector: Box<dyn Protector>) -> Self {
        Vault { dir, protector }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.dpapi"))
    }

    pub fn seal(&self, login: &Login) -> Result<String, VaultError> {
        let plain = serde_json::to_vec(login).map_err(|e| VaultError::Format(e.to_string()))?;
        let sealed = self.protector.protect(&plain)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(sealed))
    }

    pub fn unseal(&self, text: &str) -> Result<Login, VaultError> {
        let sealed = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .map_err(|e| VaultError::Format(e.to_string()))?;
        let plain = self.protector.unprotect(&sealed)?;
        let login: Login = serde_json::from_slice(&plain).map_err(|e| VaultError::Format(e.to_string()))?;
        if login.account_uuid().is_none() || !login.oauth.is_object() {
            return Err(VaultError::Format("entry lacks the account or login record".into()));
        }
        Ok(login)
    }

    pub fn store(&self, id: &str, login: &Login) -> Result<(), VaultError> {
        validate_id(id)?;
        let text = self.seal(login)?;
        atomic_write(&self.path(id), text.as_bytes())?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Login, VaultError> {
        validate_id(id)?;
        let text = fs::read_to_string(self.path(id))?;
        self.unseal(&text)
    }

    pub fn exists(&self, id: &str) -> bool {
        validate_id(id).is_ok() && self.path(id).exists()
    }

    pub fn remove(&self, id: &str) -> Result<(), VaultError> {
        validate_id(id)?;
        match fs::remove_file(self.path(id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Store a sealed backup (unknown live login before a manual switch). Returns the file.
    pub fn backup(&self, backups: &Path, tag: &str, login: &Login) -> Result<PathBuf, VaultError> {
        let text = self.seal(login)?;
        let path = backups.join(format!("{tag}-{}.dpapi", qf_model::time::now_ms()));
        atomic_write(&path, text.as_bytes())?;
        Ok(path)
    }

    pub fn load_file(&self, path: &Path) -> Result<Login, VaultError> {
        self.unseal(&fs::read_to_string(path)?)
    }
}

fn validate_id(id: &str) -> Result<(), VaultError> {
    if !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(())
    } else {
        Err(VaultError::Format("invalid vault id".into()))
    }
}

/// One PoC vault entry found for migration (`~/.quayflow-poc/vault/<name>/login.dpapi` or `login.json`).
#[derive(Debug)]
pub struct PocEntry {
    pub name: String,
    pub login: Login,
    pub was_plaintext: bool,
}

/// Read the PoC vault. Plaintext entries are read once; the caller must seal them
/// into the product vault and then delete the plaintext file.
pub fn read_poc_vault(vault: &Vault, poc_dir: &Path) -> Vec<Result<PocEntry, (String, VaultError)>> {
    let mut out = vec![];
    let Ok(entries) = fs::read_dir(poc_dir) else { return out };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('_') || !e.path().is_dir() {
            continue;
        }
        let sealed = e.path().join("login.dpapi");
        let plain = e.path().join("login.json");
        let r = if sealed.exists() {
            vault.load_file(&sealed).map(|login| PocEntry {
                name: name.clone(),
                login,
                was_plaintext: false,
            })
        } else if plain.exists() {
            fs::read_to_string(&plain)
                .map_err(VaultError::from)
                .and_then(|t| serde_json::from_str::<Login>(&t).map_err(|e| VaultError::Format(e.to_string())))
                .map(|login| PocEntry {
                    name: name.clone(),
                    login,
                    was_plaintext: true,
                })
        } else {
            continue;
        };
        out.push(r.map_err(|e| (name, e)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reversible stand-in for DPAPI (XOR) to test file handling deterministically.
    struct Fake;
    impl Protector for Fake {
        fn protect(&self, p: &[u8]) -> Result<Vec<u8>, VaultError> {
            Ok(p.iter().map(|b| b ^ 0x5a).collect())
        }
        fn unprotect(&self, s: &[u8]) -> Result<Vec<u8>, VaultError> {
            Ok(s.iter().map(|b| b ^ 0x5a).collect())
        }
    }
    struct Broken;
    impl Protector for Broken {
        fn protect(&self, _: &[u8]) -> Result<Vec<u8>, VaultError> {
            Err(VaultError::Protect("denied".into()))
        }
        fn unprotect(&self, _: &[u8]) -> Result<Vec<u8>, VaultError> {
            Err(VaultError::Protect("denied".into()))
        }
    }

    fn login() -> Login {
        Login {
            oauth: serde_json::json!({"accessToken":"sk-ant-oat-SECRET","refreshToken":"sk-ant-ort-SECRET","expiresAt":5,"subscriptionType":"team"}),
            account: serde_json::json!({"accountUuid":"u-1","organizationUuid":"o-1"}),
        }
    }

    #[test]
    fn round_trip_and_no_plaintext_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::with_protector(dir.path().to_path_buf(), Box::new(Fake));
        v.store("abc123", &login()).unwrap();
        let raw = fs::read_to_string(dir.path().join("abc123.dpapi")).unwrap();
        assert!(!raw.contains("SECRET"));
        assert_eq!(v.load("abc123").unwrap(), login());
        assert!(!format!("{:?}", login()).contains("SECRET"));
        v.remove("abc123").unwrap();
        assert!(matches!(v.load("abc123"), Err(VaultError::NotFound)));
    }

    #[test]
    fn protect_failure_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::with_protector(dir.path().to_path_buf(), Box::new(Broken));
        assert!(matches!(v.store("abc", &login()), Err(VaultError::Protect(_))));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn concurrent_writers_of_one_target_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t.json");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let target = target.clone();
                std::thread::spawn(move || {
                    let body = format!("{i}").repeat(4096);
                    for _ in 0..20 {
                        atomic_write(&target, body.as_bytes()).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let text = fs::read_to_string(&target).unwrap();
        assert_eq!(text.len(), 4096);
        assert!(
            text.chars().all(|c| c == text.chars().next().unwrap()),
            "one writer's whole content"
        );
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["t.json".to_string()], "no temp file left behind");
    }

    #[test]
    fn failed_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // The target's folder is a file: nothing can be created, and nothing is left behind.
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "x").unwrap();
        assert!(atomic_write(&blocker.join("t.json"), b"secret").is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_bad_ids() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::with_protector(dir.path().to_path_buf(), Box::new(Fake));
        assert!(v.store("..\\x", &login()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn real_dpapi_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::new(dir.path().to_path_buf());
        v.store("real", &login()).unwrap();
        assert_eq!(v.load("real").unwrap(), login());
        // Without the entropy the blob does not open (another app's DPAPI call).
        let text = fs::read_to_string(dir.path().join("real.dpapi")).unwrap();
        let blob = base64::engine::general_purpose::STANDARD.decode(text).unwrap();
        struct NoEntropy;
        impl Protector for NoEntropy {
            fn protect(&self, _: &[u8]) -> Result<Vec<u8>, VaultError> {
                unreachable!()
            }
            fn unprotect(&self, s: &[u8]) -> Result<Vec<u8>, VaultError> {
                use windows_sys::Win32::Security::Cryptography::*;
                let input = CRYPT_INTEGER_BLOB {
                    cbData: s.len() as u32,
                    pbData: s.as_ptr() as *mut u8,
                };
                let mut out = CRYPT_INTEGER_BLOB {
                    cbData: 0,
                    pbData: std::ptr::null_mut(),
                };
                let ok = unsafe {
                    CryptUnprotectData(
                        &input,
                        std::ptr::null_mut(),
                        std::ptr::null(),
                        std::ptr::null(),
                        std::ptr::null(),
                        CRYPTPROTECT_UI_FORBIDDEN,
                        &mut out,
                    )
                };
                if ok == 0 {
                    Err(VaultError::Protect("no".into()))
                } else {
                    Ok(vec![])
                }
            }
        }
        assert!(NoEntropy.unprotect(&blob).is_err());
    }

    #[test]
    fn poc_migration_reads_plaintext_and_sealed() {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::with_protector(dir.path().join("product"), Box::new(Fake));
        let poc = dir.path().join("poc");
        fs::create_dir_all(poc.join("org")).unwrap();
        fs::create_dir_all(poc.join("personal")).unwrap();
        fs::create_dir_all(poc.join("_original")).unwrap();
        fs::write(poc.join("org").join("login.dpapi"), v.seal(&login()).unwrap()).unwrap();
        fs::write(
            poc.join("personal").join("login.json"),
            serde_json::to_string(&login()).unwrap(),
        )
        .unwrap();
        let mut found = read_poc_vault(&v, &poc);
        found.sort_by_key(|r| r.as_ref().map(|e| e.name.clone()).unwrap_or_default());
        assert_eq!(found.len(), 2);
        let org = found[0].as_ref().unwrap();
        assert_eq!(org.name, "org");
        assert!(!org.was_plaintext);
        assert!(found[1].as_ref().unwrap().was_plaintext);
    }
}
