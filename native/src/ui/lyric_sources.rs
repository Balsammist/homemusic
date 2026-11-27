//! Network adapters for lyrics. All requests use the desktop's external proxy agent.
use super::details::{LyricLine, LyricWord, Lyrics, parse_lrc};
use crate::api::Track;
use serde_json::Value;
use std::time::Duration;

pub(super) const PROVIDERS: &[(&str, &str, &str)] = &[
    ("LRCLIB", "https://lrclib.net", "lyrics_lrclib"),
    ("lrc.red", "https://lrc.red", "lyrics_lrcred"),
    ("KuGou", "https://www.kugou.com", "lyrics_kugou"),
    ("SimpMusic", "https://simpmusic.org", "lyrics_simpmusic"),
];

#[derive(Clone)]
pub(super) struct Match {
    pub(super) lyrics: Lyrics,
    pub(super) source: Option<usize>,
}

impl Match {
    pub(super) fn local(lyrics: Lyrics) -> Self {
        Self {
            lyrics,
            source: None,
        }
    }
}

pub(super) fn lookup(agent: &ureq::Agent, track: &Track, prefs: &Value) -> Result<Match, String> {
    let first = prefs["lyrics_provider_first"].as_u64().unwrap_or(0) as usize;
    let prefer_words = prefs["lyrics_prefer_words"].as_bool().unwrap_or(true);
    let order: Vec<_> = (0..PROVIDERS.len())
        .filter(|i| prefs[PROVIDERS[*i].2].as_bool().unwrap_or(true))
        .collect();
    let mut order = order;
    order.sort_by_key(|i| *i != first);
    if order.is_empty() {
        return Err("Включите хотя бы один источник текста в настройках.".into());
    }

    // When comparing timing quality, independent requests finish in parallel.
    let results = if prefer_words {
        std::thread::scope(|scope| {
            order
                .iter()
                .map(|&i| (i, scope.spawn(move || from_provider(agent, track, i))))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|(i, task)| (i, task.join().unwrap_or(Err(()))))
                .collect()
        })
    } else {
        let mut results = Vec::new();
        for i in order {
            let result = from_provider(agent, track, i);
            if matches!(result, Ok(Some(Lyrics::Synced(_)))) {
                return Ok(Match {
                    lyrics: result.unwrap().unwrap(),
                    source: Some(i),
                });
            }
            results.push((i, result));
        }
        results
    };
    let mut best = None;
    let mut quality = 0;
    let mut succeeded = false;
    for (i, result) in results {
        if let Ok(result) = result {
            succeeded = true;
            if let Some(lyrics) = result {
                let rank = match &lyrics {
                    Lyrics::Synced(lines) if lines.iter().any(|line| !line.timing.is_empty()) => 3,
                    Lyrics::Synced(_) => 2,
                    Lyrics::Plain(_) => 1,
                    Lyrics::Notice(_) => 0,
                };
                if rank > quality {
                    best = Some(Match {
                        lyrics,
                        source: Some(i),
                    });
                    quality = rank;
                }
            }
        }
    }
    if let Some(best) = best {
        Ok(best)
    } else if succeeded {
        Ok(Match::local(Lyrics::Notice(
            "Текст не найден во включённых источниках.".into(),
        )))
    } else {
        Err("Источники текста недоступны. Проверьте соединение и прокси.".into())
    }
}

fn from_provider(
    agent: &ureq::Agent,
    track: &Track,
    provider: usize,
) -> Result<Option<Lyrics>, ()> {
    match provider {
        0 => lrclib(agent, track),
        1 => lrcred(agent, track),
        2 => kugou(agent, track),
        _ => simpmusic(agent, track),
    }
}

fn lrclib(agent: &ureq::Agent, track: &Track) -> Result<Option<Lyrics>, ()> {
    let mut request = agent
        .get("https://lrclib.net/api/get")
        .query("artist_name", &track.artist)
        .query("track_name", &track.title);
    if let Some(album) = track.album_title.as_deref().filter(|s| !s.is_empty()) {
        request = request.query("album_name", album);
    }
    if let Some(duration) = track.duration_s.filter(|d| *d > 0. && *d <= 3600.) {
        request = request.query("duration", duration.round().to_string());
    }
    if let Some(exact) = json(request)?.as_ref().and_then(from_json_lrc) {
        return Ok(Some(exact));
    }
    let matches = json(
        agent
            .get("https://lrclib.net/api/search")
            .query("artist_name", &track.artist)
            .query("track_name", &track.title),
    )?;
    Ok(matches
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| same_track(item, "trackName", "artistName", track))
        .find_map(from_json_lrc))
}

fn from_json_lrc(value: &Value) -> Option<Lyrics> {
    for field in ["richSyncLyrics", "syncedLyrics"] {
        if let Some(text) = value[field].as_str().and_then(synced) {
            return Some(text);
        }
    }
    value["plainLyrics"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(|text| Lyrics::Plain(text.into()))
}

fn lrcred(agent: &ureq::Agent, track: &Track) -> Result<Option<Lyrics>, ()> {
    let value = json(
        agent
            .get("https://lrc.red/api/v1")
            .query("q", format!("{} {}", track.title, track.artist)),
    )?;
    let best = value
        .as_ref()
        .and_then(|v| v["results"].as_array())
        .into_iter()
        .flatten()
        .filter(|item| same_track(item, "track_name", "artist_name", track))
        .max_by_key(|item| {
            item["timing_type"]
                .as_str()
                .is_some_and(|v| v.contains("word"))
        });
    let Some(url) = best.and_then(|v| v["lyricsUrl"].as_str()) else {
        return Ok(None);
    };
    let url = if url.starts_with('/') && !url.starts_with("//") {
        format!("https://lrc.red{url}")
    } else if url.starts_with("https://lrc.red/") {
        url.to_string()
    } else {
        return Ok(None);
    };
    let Some(text) = body(agent.get(&url))? else {
        return Ok(None);
    };
    let lines = parse_ttml(&text);
    Ok((!lines.is_empty()).then_some(Lyrics::Synced(lines)))
}

fn same_track(item: &Value, title: &str, artist: &str, track: &Track) -> bool {
    fn normalized(value: &str) -> String {
        value
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect()
    }
    item[title]
        .as_str()
        .is_some_and(|name| normalized(name) == normalized(&track.title))
        && item[artist]
            .as_str()
            .is_some_and(|name| normalized(name) == normalized(&track.artist))
        && track
            .duration_s
            .zip(item["duration"].as_f64())
            .is_none_or(|(a, b)| {
                let seconds = if b > 10_000. { b / 1000. } else { b };
                (a - seconds).abs() <= 8.
            })
}

fn kugou(agent: &ureq::Agent, track: &Track) -> Result<Option<Lyrics>, ()> {
    let mut request = agent
        .get("https://lyrics.kugou.com/search")
        .query("ver", "1")
        .query("man", "yes")
        .query("client", "pc")
        .query("keyword", format!("{} - {}", track.artist, track.title));
    if let Some(seconds) = track.duration_s {
        request = request.query("duration", format!("{:.0}", seconds * 1000.));
    }
    let value = json(request)?;
    let Some(item) = value
        .as_ref()
        .and_then(|v| v["candidates"].as_array())
        .into_iter()
        .flatten()
        .find(|item| same_track(item, "song", "singer", track))
    else {
        return Ok(None);
    };
    let Some(key) = item["accesskey"].as_str() else {
        return Ok(None);
    };
    let id = item["id"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| item["id"].as_u64().map(|id| id.to_string()));
    let Some(id) = id else { return Ok(None) };
    let value = json(
        agent
            .get("https://lyrics.kugou.com/download")
            .query("fmt", "lrc")
            .query("charset", "utf8")
            .query("client", "pc")
            .query("ver", "1")
            .query("id", id)
            .query("accesskey", key),
    )?;
    let Some(encoded) = value.as_ref().and_then(|v| v["content"].as_str()) else {
        return Ok(None);
    };
    let bytes = gtk::glib::base64_decode(encoded);
    let text = std::str::from_utf8(bytes.as_ref()).map_err(|_| ())?;
    Ok(synced(text.strip_prefix('\u{feff}').unwrap_or(text)))
}

fn simpmusic(agent: &ureq::Agent, track: &Track) -> Result<Option<Lyrics>, ()> {
    let Some(id) = track
        .playback_ref
        .iter()
        .chain([&track.reference])
        .find_map(|reference| {
            ["file:youtube:", "file:yt:", "youtube:", "yt:"]
                .iter()
                .find_map(|prefix| reference.strip_prefix(prefix))
        })
        .filter(|id| {
            id.len() == 11
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
    else {
        return Ok(None);
    };
    let value = json(
        agent
            .get(&format!("https://api-lyrics.simpmusic.org/v1/{id}"))
            .header("User-Agent", "SimpMusicLyrics/1.0")
            .header("Accept", "application/json"),
    )?;
    Ok(value
        .as_ref()
        .and_then(|v| v["data"].as_array())
        .into_iter()
        .flatten()
        .filter(|item| {
            track
                .duration_s
                .zip(item["durationSeconds"].as_f64())
                .is_none_or(|(a, b)| (a - b).abs() <= 10.)
        })
        .find_map(from_json_lrc))
}

fn synced(text: &str) -> Option<Lyrics> {
    let lines = parse_lrc(text);
    (!lines.is_empty()).then_some(Lyrics::Synced(lines))
}

fn json(request: ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Result<Option<Value>, ()> {
    body(request)?
        .map(|value| serde_json::from_str(&value).map_err(|_| ()))
        .transpose()
}

fn body(request: ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Result<Option<String>, ()> {
    let mut response = match request
        .config()
        .timeout_global(Some(Duration::from_secs(6)))
        .build()
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404 | 204)) => return Ok(None),
        Err(_) => return Err(()),
    };
    response
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map(Some)
        .map_err(|_| ())
}

fn parse_ttml(text: &str) -> Vec<LyricLine> {
    let Ok(document) = roxmltree::Document::parse(text) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let mut first_agent = None;
    for paragraph in document.descendants().filter(|node| node.has_tag_name("p")) {
        let Some(seconds) = paragraph.attribute("begin").and_then(timestamp) else {
            continue;
        };
        let agent = paragraph.ancestors().find_map(|node| {
            node.attributes()
                .find(|attr| attr.name() == "agent")
                .map(|attr| attr.value())
        });
        if first_agent.is_none() {
            first_agent = agent;
        }
        let alternate = agent.zip(first_agent).is_some_and(|(a, b)| a != b);
        let mut words = String::new();
        let mut timing = Vec::new();
        let mut owner = None;
        for node in paragraph.descendants().filter(|node| node.is_text()) {
            if node.ancestors().any(|node| {
                node.attributes().any(|a| {
                    a.name() == "role" && matches!(a.value(), "x-translation" | "x-roman" | "x-bg")
                })
            }) {
                continue;
            }
            let value = node.text().unwrap_or_default();
            words.push_str(value);
            if let Some(span) = node
                .ancestors()
                .find(|n| n.has_tag_name("span") && n.attribute("begin").is_some())
                && let Some(begin) = span.attribute("begin").and_then(timestamp)
            {
                if owner == Some(span) {
                    if let Some(word) = timing.last_mut() {
                        let word: &mut LyricWord = word;
                        word.byte_end = words.len();
                    }
                } else {
                    timing.push(LyricWord {
                        begin,
                        end: span
                            .attribute("end")
                            .and_then(timestamp)
                            .filter(|end| *end >= begin),
                        byte_end: words.len(),
                    });
                    owner = Some(span);
                }
            }
        }
        if !words.trim().is_empty() {
            lines.push(LyricLine {
                seconds,
                words,
                timing,
                alternate,
                end: paragraph.attribute("end").and_then(timestamp),
            });
        }
    }
    lines.sort_by(|a, b| a.seconds.total_cmp(&b.seconds));
    lines
}

fn timestamp(value: &str) -> Option<f64> {
    if let Some(milliseconds) = value.strip_suffix("ms") {
        return Some(milliseconds.parse::<f64>().ok()? / 1000.)
            .filter(|s| s.is_finite() && *s >= 0.);
    }
    let time = value
        .trim_end_matches('s')
        .split(':')
        .try_fold(0., |sum, term| Some(sum * 60. + term.parse::<f64>().ok()?))?;
    Some(time).filter(|s| s.is_finite() && *s >= 0.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_lrc_retains_utf8_ranges_and_absolute_word_intervals() {
        let lines = parse_lrc("[00:10.00]<00:10.00>Привет <00:10.70>мир<00:11.25>");
        assert_eq!(lines[0].words, "Привет мир");
        assert_eq!(lines[0].timing[0].byte_end, "Привет ".len());
        assert_eq!(lines[0].timing[1].end, Some(11.25));
        assert_eq!(lines[0].end, Some(11.25));
    }

    #[test]
    #[ignore = "requires external lyric providers and configured proxy"]
    fn live_lookup_each_metadata_provider() {
        let track = Track {
            title: "Creep".into(),
            artist: "Radiohead".into(),
            duration_s: Some(238.),
            playback_ref: Some("file:youtube:XFkzRNyygfk".into()),
            ..Track::default()
        };
        let agent = super::super::details::external_agent();
        for index in 0..PROVIDERS.len() {
            let lyrics = from_provider(&agent, &track, index);
            assert!(
                matches!(lyrics, Ok(Some(Lyrics::Synced(_) | Lyrics::Plain(_)))),
                "provider {}",
                PROVIDERS[index].0
            );
            if index == 1 {
                assert!(
                    matches!(lyrics, Ok(Some(Lyrics::Synced(lines))) if lines.iter().any(|line| !line.timing.is_empty()))
                );
            }
        }
    }

    #[test]
    fn ttml_keeps_word_times_vocal_agents_and_escaped_text() {
        let xml = r#"<tt xmlns:ttm="urn:ttm"><body><div><p ttm:agent="v1" begin="1s" end="2s"><span begin="1s" end="1.4s">A &amp;</span> <span begin="1.5s" end="2s">Б</span></p><p ttm:agent="v2" begin="2s" end="3s">B</p></div></body></tt>"#;
        let lines = parse_ttml(xml);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words, "A & Б");
        assert_eq!(lines[0].timing[1].byte_end, "A & Б".len());
        assert_eq!(lines[0].timing[0].end, Some(1.4));
        assert!(lines[1].alternate);
    }
}
