use serde_json::{Value, json};
use std::time::Duration;

pub(super) const LANGUAGES: &[&str] = &[
    "Русский",
    "Английский",
    "Немецкий",
    "Французский",
    "Испанский",
    "Итальянский",
    "Португальский",
    "Украинский",
    "Японский",
    "Корейский",
    "Китайский",
];
const CODES: &[&str] = &[
    "RU", "EN-US", "DE", "FR", "ES", "IT", "PT-BR", "UK", "JA", "KO", "ZH-HANS",
];

pub(super) fn translate(agent: &ureq::Agent, lines: &[String]) -> Result<Vec<String>, String> {
    let settings = Value::Object(crate::config::read_settings().unwrap_or_default());
    let key = settings["lyrics_deepl_key"]
        .as_str()
        .filter(|key| !key.is_empty())
        .ok_or("Введите ключ DeepL в настройках текста.")?;
    let url = if key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };
    let language = settings["lyrics_translate_language"].as_u64().unwrap_or(0) as usize;
    let target = CODES.get(language).copied().unwrap_or(CODES[0]);
    let format = settings["lyrics_translate_formality"].as_u64().unwrap_or(0);
    let formality = match format {
        1 => "prefer_less",
        2 => "prefer_more",
        _ => "default",
    };
    let mut translated = vec![String::new(); lines.len()];
    let requested: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    for chunk in requested.chunks(30) {
        let text: Vec<_> = chunk.iter().map(|(_, text)| text.as_str()).collect();
        let body = json!({ "text": text, "target_lang": target, "formality": formality });
        let size = serde_json::to_vec(&body)
            .map_err(|_| "Не удалось подготовить текст.")?
            .len();
        if size > 120 * 1024 {
            return Err("Слишком большой фрагмент для перевода.".into());
        }
        let mut response = agent
            .post(url)
            .header("Authorization", &format!("DeepL-Auth-Key {key}"))
            .config()
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .send_json(body)
            .map_err(|error| match error {
                ureq::Error::StatusCode(403) => "DeepL отклонил ключ API.",
                ureq::Error::StatusCode(429 | 456) => "DeepL ограничил запросы или исчерпал квоту.",
                _ => "Не удалось получить перевод от DeepL.",
            })?;
        let payload: Value = response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_json()
            .map_err(|_| "DeepL вернул некорректный ответ.")?;
        let results = payload["translations"]
            .as_array()
            .filter(|items| items.len() == chunk.len())
            .ok_or("DeepL вернул неполный перевод.")?;
        for ((index, _), result) in chunk.iter().zip(results) {
            translated[*index] = result["text"]
                .as_str()
                .ok_or("Некорректная строка перевода.")?
                .into();
        }
    }
    Ok(translated)
}
