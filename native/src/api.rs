use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Track {
    #[serde(rename = "ref", default)]
    pub reference: String,
    #[serde(default)]
    pub playback_ref: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_full_audio: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album_title: Option<String>,
    #[serde(default)]
    pub duration_s: Option<f64>,
    #[serde(default)]
    pub cover_small_url: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub nd_song_id: Option<String>,
    #[serde(default)]
    pub owned: bool,
    #[serde(default)]
    pub nd_album_id: Option<String>,
    #[serde(default)]
    pub nd_artist_id: Option<String>,
    #[serde(default)]
    pub album_ref: Option<String>,
    #[serde(default)]
    pub artist_ref: Option<String>,
    #[serde(default)]
    pub liked: bool,
}
impl Track {
    pub fn cover(&self) -> String {
        self.cover_small_url
            .clone()
            .filter(|s| !s.is_empty())
            .or(self.cover_url.clone().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| {
                if self.reference.is_empty() {
                    String::new()
                } else {
                    format!("metadata:{}", self.reference)
                }
            })
    }
}
#[derive(Clone)]
pub struct Api {
    pub agent: ureq::Agent,
    pub base: String,
    cookie: Arc<Mutex<String>>,
    image_scope: Arc<Mutex<String>>,
}
impl Api {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(100)))
            .input_buffer_size(16 * 1024)
            .output_buffer_size(8 * 1024)
            .max_idle_connections(4)
            .max_idle_connections_per_host(2)
            .build();
        Self {
            agent: config.into(),
            base: crate::config::backend(),
            cookie: Arc::default(),
            image_scope: Arc::default(),
        }
    }
    pub fn login(&self) -> Result<(), String> {
        let env = if crate::config::test() {
            "NAVIDROME_ADMIN_USER=fixture\nNAVIDROME_ADMIN_PASSWORD=fixture".to_string()
        } else {
            fs::read_to_string(crate::config::root().join(".env"))
                .map_err(|_| "Не найден локальный файл подключения".to_string())?
        };
        let value = |key: &str| {
            env.lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .unwrap_or("")
                .to_string()
        };
        let mut r = self.agent.post(format!("{}/api/auth/login", self.base))
            .config()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .send_json(json!({"username":value("NAVIDROME_ADMIN_USER"),"password":value("NAVIDROME_ADMIN_PASSWORD")}))
            .map_err(|_| "Не удалось войти в HomeMusic. Проверьте, что сервер запущен и пароль в .env актуален.".to_string())?;
        let cookie = r
            .headers()
            .get("set-cookie")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.split(';').next())
            .unwrap_or("")
            .to_string();
        if !cookie.starts_with("hm_session=") {
            return Err("Сервер не выдал сеанс входа".into());
        }
        let _: Value = r
            .body_mut()
            .read_json()
            .map_err(|_| "Некорректный ответ входа")?;
        *self.cookie.lock().unwrap() = cookie;
        *self.image_scope.lock().unwrap() = value("NAVIDROME_ADMIN_USER");
        Ok(())
    }
    pub fn profile_scope(&self) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let profile = self.image_scope.lock().unwrap();
        if profile.is_empty() {
            return None;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.base.hash(&mut hasher);
        profile.hash(&mut hasher);
        Some(format!("{:016x}", hasher.finish()))
    }
    pub fn cookie(&self) -> String {
        self.cookie.lock().unwrap().clone()
    }
    pub fn get(&self, path: &str) -> Result<Value, String> {
        self.get_with_timeout(path, Duration::from_secs(100))
    }
    pub fn get_with_timeout(&self, path: &str, timeout: Duration) -> Result<Value, String> {
        let mut r = self
            .agent
            .get(format!("{}{path}", self.base))
            .config()
            .timeout_global(Some(timeout))
            .build()
            .header("Cookie", self.cookie())
            .call()
            .map_err(error)?;
        r.body_mut().read_json().map_err(error)
    }
    pub fn query(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        self.query_with_timeout(path, params, Duration::from_secs(100))
    }
    pub fn query_with_timeout(
        &self, path: &str, params: &[(&str, &str)], timeout: Duration,
    ) -> Result<Value, String> {
        let mut req = self
            .agent
            .get(format!("{}{path}", self.base))
            .config()
            .timeout_global(Some(timeout))
            .build()
            .header("Cookie", self.cookie());
        for (k, v) in params {
            req = req.query(k, v);
        }
        let mut r = req.call().map_err(error)?;
        r.body_mut().read_json().map_err(error)
    }
    pub fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        let mut r = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("Cookie", self.cookie())
            .send_json(body)
            .map_err(error)?;
        r.body_mut().read_json().map_err(error)
    }
    pub fn mutate(&self, method: &str, path: &str, body: Value) -> Result<Value, String> {
        let mut r = match method {
            "DELETE" => self
                .agent
                .delete(format!("{}{path}", self.base))
                .header("Cookie", self.cookie())
                .call()
                .map_err(error)?,
            "PATCH" => self
                .agent
                .patch(format!("{}{path}", self.base))
                .header("Cookie", self.cookie())
                .send_json(body)
                .map_err(error)?,
            _ => return Err("Неизвестное действие".into()),
        };
        r.body_mut().read_json().map_err(error)
    }
    pub fn upload(&self, path: &std::path::Path) -> Result<Value, String> {
        let meta = fs::metadata(path).map_err(error)?;
        if meta.len() > 50 * 1024 * 1024 {
            return Err("Экспорт больше 50 МБ. Выберите отдельные файлы библиотеки.".into());
        }
        let bytes = fs::read(path).map_err(error)?;
        let mut r = self
            .agent
            .post(format!("{}/api/imports/spotify/export", self.base))
            .query(
                "filename",
                path.file_name().unwrap_or_default().to_string_lossy(),
            )
            .header("Cookie", self.cookie())
            .header("Content-Type", "application/octet-stream")
            .send(bytes.as_slice())
            .map_err(error)?;
        r.body_mut().read_json().map_err(error)
    }
    pub fn image(&self, url: &str) -> Result<Vec<u8>, String> {
        if url.is_empty() {
            return Err("empty".into());
        }
        if let Some(reference) = url.strip_prefix("metadata:") {
            let mut response = self
                .agent
                .get(format!("{}/api/catalog/track/{reference}", self.base))
                .header("Cookie", self.cookie())
                .config()
                .timeout_global(Some(Duration::from_secs(8)))
                .build()
                .call()
                .map_err(error)?;
            let value: Value = response.body_mut().read_json().map_err(error)?;
            let cover = value["cover_small_url"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| value["cover_url"].as_str().filter(|s| !s.is_empty()))
                .ok_or_else(|| "Обложка пока отсутствует в каталоге".to_string())?;
            return self.image(cover);
        }
        let url = if url.starts_with('/') {
            format!("{}{url}", self.base)
        } else {
            url.into()
        };
        if crate::config::test() && !url_is_local(&url) {
            return Err("external images disabled in fixture tests".into());
        }
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut hash);
        self.base.hash(&mut hash);
        self.image_scope.lock().unwrap().hash(&mut hash); // Stable account scope across fresh login sessions.
        let directory = if crate::config::test() {
            crate::config::test_root().join("artwork")
        } else {
            crate::config::root().join("cache/artwork")
        };
        let cached = directory.join(format!("{:016x}.img", hash.finish()));
        if let Ok(meta) = fs::metadata(&cached) {
            if meta
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() < 30 * 86400)
            {
                if let Ok(bytes) = fs::read(&cached) {
                    return Ok(bytes);
                }
            }
        }
        let mut r = self
            .agent
            .get(&url)
            .header(
                "Cookie",
                if url_is_local(url.as_str()) {
                    self.cookie()
                } else {
                    String::new()
                },
            )
            .config()
            .timeout_global(Some(Duration::from_secs(12)))
            .build()
            .call()
            .map_err(error)?;
        let bytes = r
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .read_to_vec()
            .map_err(error)?;
        static CACHE_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _cache_guard = CACHE_WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let existed_directory = directory.exists();
        if fs::create_dir_all(&directory).is_ok() {
            if !existed_directory {
                crate::config::record_change("CREATE", &directory, "artwork disk cache");
            }
            // Opportunistic disk LRU, independent of the bounded decoded texture cache.
            let mut files: Vec<_> = fs::read_dir(&directory)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    e.metadata()
                        .ok()
                        .map(|m| (e.path(), m.len(), m.modified().ok()))
                })
                .collect();
            let mut total: u64 = files.iter().map(|f| f.1).sum();
            files.sort_by_key(|f| f.2);
            for (path, size, _) in files {
                if total + bytes.len() as u64 <= 128 * 1024 * 1024 {
                    break;
                }
                if fs::remove_file(&path).is_ok() {
                    total = total.saturating_sub(size);
                    crate::config::record_change("DELETE", &path, "artwork cache eviction");
                }
            }
            let existed = cached.exists();
            if fs::write(&cached, &bytes).is_ok() {
                crate::config::record_change(
                    if existed { "MODIFY" } else { "CREATE" },
                    &cached,
                    "artwork disk cache",
                );
            }
        }
        Ok(bytes)
    }
}
fn url_is_local(url: &str) -> bool {
    url.starts_with(&format!("{}/", crate::config::backend()))
}
fn error(e: impl std::fmt::Display) -> String {
    let s = e.to_string();
    if s.contains("401") {
        "Сеанс истёк. Повторите подключение в настройках.".into()
    } else if s.contains("404") {
        "404: запись или сессия больше недоступна".into()
    } else if s.contains("400") || s.contains("422") {
        "Сервер не принял запрос. Проверьте выбранный файл или действие.".into()
    } else {
        "Не удалось получить ответ сервера. Проверьте HomeMusic и прокси.".into()
    }
}
pub fn tracks(v: &Value) -> Vec<Track> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let mut row = row.clone();
            if row["ref"].as_str().is_none_or(|s| s.is_empty()) {
                if let Some(id) = row["nd_song_id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                {
                    row["ref"] = json!(format!("nd:{id}"));
                    row["owned"] = json!(true);
                }
            }
            if row["album_title"].is_null() {
                if let Some(album) = row["album"].as_str().map(String::from) {
                    row["album_title"] = json!(album);
                }
            }
            serde_json::from_value(row).ok()
        })
        .collect()
}

#[cfg(test)]
mod track_shape_tests {
    use super::*;
    #[test]
    fn library_search_uses_real_navidrome_identity_and_album() {
        let rows = tracks(
            &json!([{"id":42,"nd_song_id":"actual-song","title":"Local song","artist":"Artist","album":"Actual album"}]),
        );
        assert_eq!(rows[0].reference, "nd:actual-song");
        assert_eq!(rows[0].album_title.as_deref(), Some("Actual album"));
        assert!(rows[0].owned);
    }
    #[test]
    fn catalog_identity_survives_owned_playback_mapping() {
        let rows =
            tracks(&json!([{"ref":"dz:77","nd_song_id":"local-copy","title":"Canonical song"}]));
        assert_eq!(rows[0].reference, "dz:77");
    }
    #[test]
    fn malformed_record_cannot_erase_other_results_or_invent_identity() {
        let rows = tracks(
            &json!([null,{"title":12},{"id":99,"title":"Not scanned yet"},{"ref":"dz:1","title":"Valid"}]),
        );
        assert_eq!(rows.len(), 2);
        assert!(rows[0].reference.is_empty());
        assert_eq!(rows[1].reference, "dz:1");
    }
}
