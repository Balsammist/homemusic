//! Test state never falls back to production files or credentials.
use std::{path::PathBuf, sync::OnceLock};

/// The launcher sets this to the checkout. Direct binary launches can set it explicitly.
pub fn root() -> &'static std::path::Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::var_os("HOMEMUSIC_ROOT")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().expect("HomeMusic working directory"))
    })
}

pub fn test() -> bool {
    std::env::var_os("HOMEMUSIC_TEST").is_some()
}
pub fn test_root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        use std::os::unix::fs::DirBuilderExt;
        let fixture_root = std::env::var_os("HOMEMUSIC_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!(
                    "homemusic-native-test-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ))
            });
        assert!(fixture_root.is_absolute(), "HOMEMUSIC_TEST_ROOT must be absolute");
        // Require a fresh directory: no accidental reuse of another instance's state/socket.
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&fixture_root)
            .expect("test root must be a new directory under an existing parent");
        if let Some(seed) = std::env::var_os("HOMEMUSIC_TEST_SEED_SETTINGS") {
            let seed = PathBuf::from(seed)
                .canonicalize()
                .expect("fixture settings seed");
            assert!(seed.starts_with(root().join("evidence")));
            std::fs::copy(&seed, fixture_root.join("settings.json")).expect("copy fixture settings");
            let source = seed.with_file_name("collections");
            if let Ok(entries) = std::fs::read_dir(source) {
                let target = fixture_root.join("collections");
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&target)
                    .expect("fixture collection cache");
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|kind| kind.is_file())
                        && entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "json")
                    {
                        std::fs::copy(entry.path(), target.join(entry.file_name()))
                            .expect("copy fixture collection cache");
                    }
                }
            }
        }
        fixture_root
    })
}
pub fn settings_path() -> PathBuf {
    if test() {
        test_root().join("settings.json")
    } else {
        root().join("native/settings.json")
    }
}
type Preferences = serde_json::Map<String, serde_json::Value>;

fn with_settings<T>(read: impl FnOnce(&Preferences) -> T) -> std::io::Result<T> {
    use std::{os::unix::fs::MetadataExt, sync::Mutex};
    type Version = (u64, u64, i64, i64, u64);
    static CACHE: Mutex<Option<(PathBuf, Version, Preferences)>> = Mutex::new(None);
    let path = settings_path();
    let mut cache = CACHE.lock().unwrap();
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            *cache = None;
            return Ok(read(&Preferences::new()));
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    let version = (
        metadata.dev(),
        metadata.ino(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.len(),
    );
    if let Some((_, _, settings)) = cache
        .as_ref()
        .filter(|(p, v, _)| *v == version && p == &path)
    {
        return Ok(read(settings));
    }
    let settings = serde_json::from_reader(file).map_err(std::io::Error::other)?;
    let result = read(&settings);
    *cache = Some((path, version, settings));
    Ok(result)
}

pub fn read_settings() -> std::io::Result<Preferences> {
    with_settings(Clone::clone)
}

pub fn preference(key: &str) -> serde_json::Value {
    with_settings(|settings| settings.get(key).cloned().unwrap_or_default()).unwrap_or_default()
}

pub fn update_settings(
    values: &[(&str, serde_json::Value)],
    remove: &[&str],
) -> std::io::Result<()> {
    use std::{fs, io::Write, os::unix::fs::OpenOptionsExt};
    static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITE_LOCK.lock().unwrap();
    let path = settings_path();
    let mut settings = read_settings()?;
    let original = settings.clone();
    for key in remove {
        settings.remove(*key);
    }
    for (key, value) in values {
        settings.insert((*key).into(), value.clone());
    }
    if settings == original {
        return Ok(());
    }
    let payload = serde_json::to_vec_pretty(&settings).map_err(std::io::Error::other)?;
    let existed = path.exists();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("{}-{stamp}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    record_change("CREATE", &temporary, "Atomic HomeMusic settings update");
    let result = (|| {
        file.write_all(&payload)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    match result {
        Ok(()) => {
            record_change("MOVE", &temporary, "Replaced settings file atomically");
            record_change(
                if existed { "MODIFY" } else { "CREATE" },
                &path,
                "HomeMusic settings and playback state",
            );
        }
        Err(_) if fs::remove_file(&temporary).is_ok() => {
            record_change("DELETE", &temporary, "Discard failed settings update");
        }
        Err(_) => (),
    }
    result
}
pub fn pin_path() -> PathBuf {
    if test() {
        test_root().join("source-pins.json")
    } else {
        std::env::var_os("HOMEMUSIC_PIN_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root().join("native/source-pins.json")
            })
    }
}
pub fn socket_path() -> PathBuf {
    test_root().join("control.sock")
}
pub fn backend() -> String {
    if !test() {
        return std::env::var("HOMEMUSIC_API_URL")
            .ok()
            .map(|url| url.trim().trim_end_matches('/').to_owned())
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| "http://127.0.0.1:8080".into());
    }
    let base = std::env::var("HOMEMUSIC_TEST_API").unwrap_or_else(|_| "http://127.0.0.1:9".into());
    assert!(
        fixture_backend(&base),
        "test backend must be a loopback fixture on a port other than 8080"
    );
    base
}
fn fixture_backend(base: &str) -> bool {
    base.strip_prefix("http://127.0.0.1:")
        .and_then(|port| port.parse::<u16>().ok())
        .is_some_and(|port| port != 0 && port != 8080)
}
/// Append-only mutation audit; fixture runs keep their log within their owned root.
pub fn record_change(operation: &str, path: &std::path::Path, reason: &str) {
    use std::io::Write;
    let log = if test() {
        test_root().join("CHANGELOG.md")
    } else {
        std::env::var_os("HOMEMUSIC_CHANGELOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| root().join("CHANGELOG.md"))
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(log)
    {
        let timestamp = gtk::glib::DateTime::now_utc()
            .ok()
            .and_then(|t| t.format_iso8601().ok())
            .map(|t| t.to_string())
            .unwrap_or_default();
        let _ = writeln!(
            file,
            "{} {} {} — {}",
            timestamp,
            operation,
            path.display(),
            reason
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_paths_do_not_touch_production() {
        if !test() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "config::tests::isolated_paths_do_not_touch_production",
                ])
                .env("HOMEMUSIC_TEST", "1")
                .env_remove("HOMEMUSIC_TEST_ROOT")
                .env_remove("HOMEMUSIC_TEST_API")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        let root = test_root();
        assert_eq!(backend(), "http://127.0.0.1:9");
        for path in [settings_path(), pin_path(), socket_path()] {
            assert_eq!(path.parent(), Some(root.as_path()));
            assert!(!path.exists());
        }
        std::fs::write(settings_path(), br#"{"motion":2}"#).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(settings_path()).unwrap()).unwrap();
        assert_eq!(saved["motion"], 2);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn backend_rejects_production_and_nonlocal_urls() {
        for base in [
            "http://127.0.0.1:8080",
            "http://example.com:1234",
            "http://127.0.0.1:1234/api",
            "http://127.0.0.1:0",
        ] {
            assert!(!fixture_backend(base), "{base}");
        }
        assert!(fixture_backend("http://127.0.0.1:19080"));
    }
}
