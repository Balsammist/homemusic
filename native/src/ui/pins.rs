use super::*;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Pin {
    pub reference: String,
    pub provider: String,
    pub duration_s: Option<f64>,
}
pub(super) fn load() -> HashMap<String, Pin> {
    std::fs::read(pin_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
impl Ui {
    pub(super) fn pinned(&self, mut t: Track) -> Track {
        if self.stars_loaded.get() {
            t.liked = self.stars.borrow().contains(&t.reference)
                || t.nd_song_id
                    .as_ref()
                    .is_some_and(|id| self.stars.borrow().contains(&format!("nd:{id}")));
        }
        if let Some(pin) = self.pins.borrow().get(&t.reference) {
            t.playback_ref = Some(pin.reference.clone());
            if let Some(duration) = pin.duration_s {
                t.duration_s = Some(duration);
            }
        }
        t
    }
    pub(super) fn pin_source(self: &Rc<Self>, track: &Track, pin: Pin) {
        self.pins
            .borrow_mut()
            .insert(track.reference.clone(), pin.clone());
        {
            use std::os::unix::fs::PermissionsExt;
            let path = pin_path();
            if let Ok(bytes) = serde_json::to_vec_pretty(&*self.pins.borrow()) {
                let temp = format!("{path}.new");
                if std::fs::write(&temp, bytes).is_ok() {
                    let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600));
                    let _ = std::fs::rename(temp, path);
                }
            }
        }
        for t in self.queue.borrow_mut().iter_mut() {
            if t.reference == track.reference {
                *t = self.pinned(t.clone());
            }
        }
        for t in self.queue_original.borrow_mut().iter_mut() {
            if t.reference == track.reference {
                *t = self.pinned(t.clone());
            }
        }
        let items = self.items.borrow().clone();
        self.set_items(items);
        let index = self.index.get();
        if self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|t| t.reference == track.reference)
            && index < self.queue.borrow().len()
        {
            self.play_index(index);
        } else {
            *self.queue.borrow_mut() = vec![self.pinned(track.clone())];
            self.play_index(0);
        }
    }
}

fn pin_path() -> String {
    crate::config::pin_path().to_string_lossy().into_owned()
}

// Explicit decisions override stale provider stars, including all known identities.
pub(super) fn favorite_refs(v: &Value) -> HashSet<String> {
    let mut refs = HashSet::new();
    for key in ["refs", "song_ids"] {
        for id in v[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            refs.insert(if key == "song_ids" {
                format!("nd:{id}")
            } else {
                id.to_string()
            });
        }
    }
    for subject in v["subjects"].as_array().into_iter().flatten() {
        if subject["target_type"] != "track" {
            continue;
        }
        let mut aliases = Vec::new();
        if let Some(r) = subject["ref"].as_str() {
            aliases.push(r.to_string());
        }
        if let Some(id) = subject["nd_id"].as_str() {
            aliases.push(format!("nd:{id}"));
        }
        for alias in subject["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            aliases.push(alias.strip_prefix("ref:").unwrap_or(alias).to_string());
        }
        for alias in aliases {
            if subject["liked"] == true {
                refs.insert(alias);
            } else {
                refs.remove(&alias);
            }
        }
    }
    refs
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_unlike_overrides_provider_star_aliases() {
        let v = json!({"refs":["dz:1"],"song_ids":["abc"],"subjects":[{"target_type":"track","ref":"dz:1","nd_id":"abc","liked":false,"aliases":["ref:dz:1","nd:abc"]}]});
        assert!(favorite_refs(&v).is_empty());
    }
    #[test]
    fn pending_import_likes_are_present_without_downloads() {
        let v = json!({"subjects":[{"target_type":"track","ref":"dz:2","liked":true,"aliases":["ref:dz:2"]}]});
        assert!(favorite_refs(&v).contains("dz:2"));
    }
}
