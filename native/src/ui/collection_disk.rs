use super::*;
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub(super) fn path(api: &Api, route: &str) -> Option<PathBuf> {
    let scope = api.profile_scope()?;
    let directory = if crate::config::test() {
        crate::config::test_root().join("collections")
    } else {
        crate::config::root().join("cache/native-collections")
    };
    Some(directory.join(format!("{scope}-{route}.json")))
}

pub(super) fn load(path: &Path) -> Option<Value> {
    let file = fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if metadata.len() > MAX_BYTES || metadata.modified().ok()?.elapsed().ok()? > MAX_AGE {
        return None;
    }
    serde_json::from_reader(file).ok()
}

pub(super) fn snapshot(route: &str, value: Value) -> Value {
    if route == "likes" {
        return json!({"songs": api::tracks(&value["songs"])});
    }
    let rows: Vec<_> = value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["status"] == "done")
        .map(|row| {
            json!({
                "status": "done", "nd_song_id": row["nd_song_id"], "track_ref": row["track_ref"],
                "title": row["title"], "artist": row["artist"], "album": row["album"],
            })
        })
        .collect();
    json!(rows)
}

pub(super) fn save(path: &Path, snapshot: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(snapshot).map_err(std::io::Error::other)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Ok(());
    }
    let directory = path.parent().expect("collection cache directory");
    match fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => crate::config::record_change("CREATE", directory, "Private collection cache"),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = path.with_extension(format!("{}-{stamp}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    crate::config::record_change("CREATE", &temp, "Prepare collection cache snapshot");
    let existed = path.exists();
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_ok() {
        crate::config::record_change("MOVE", &temp, "Publish collection cache snapshot");
        crate::config::record_change(
            if existed { "MODIFY" } else { "CREATE" },
            path,
            "Collection cache snapshot",
        );
    } else if fs::remove_file(&temp).is_ok() {
        crate::config::record_change("DELETE", &temp, "Discard failed collection cache snapshot");
    }
    result
}

pub(super) fn forget(path: &Path) {
    if fs::remove_file(path).is_ok() {
        crate::config::record_change(
            "DELETE",
            path,
            "Invalidate collection cache after favorite change",
        );
    }
}
