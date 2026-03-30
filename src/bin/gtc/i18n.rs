use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use greentic_i18n::normalize_locale;
use gtc::error::{GtcError, GtcResult};
use serde_json::Value;

const LOCALES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/i18n/locales.json"
));
include!(concat!(env!("OUT_DIR"), "/embedded_i18n.rs"));

pub(super) fn t(locale: &str, key: &'static str) -> Cow<'static, str> {
    Cow::Owned(i18n().translate(locale, key))
}

pub(super) fn t_or(locale: &str, key: &'static str, fallback: &'static str) -> String {
    let value = t(locale, key).into_owned();
    if value == key {
        fallback.to_string()
    } else {
        value
    }
}

pub(super) fn tf(locale: &str, key: &'static str, replacements: &[(&str, &str)]) -> String {
    let mut value = t(locale, key).into_owned();
    for (name, replace) in replacements {
        let token = format!("{{{name}}}");
        value = value.replace(&token, replace);
    }
    value
}

// clap's builder APIs still want &'static str for several fields, so we keep
// these localized strings alive for the whole CLI process.
pub(super) fn leak_str(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

pub(super) fn i18n() -> &'static I18nCatalog {
    static CATALOG: OnceLock<I18nCatalog> = OnceLock::new();
    CATALOG.get_or_init(I18nCatalog::load)
}

#[derive(Debug)]
pub(super) struct I18nCatalog {
    default_locale: String,
    supported: HashSet<String>,
    dictionaries: HashMap<String, HashMap<String, String>>,
}

impl I18nCatalog {
    fn load() -> Self {
        let locales: Value = serde_json::from_str(LOCALES_JSON).expect("valid locales.json");
        let default_locale = locales
            .get("default")
            .and_then(Value::as_str)
            .unwrap_or("en")
            .to_string();

        let supported = locales
            .get("supported")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(normalize_locale)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_else(|| {
                let mut set = HashSet::new();
                set.insert(normalize_locale(&default_locale));
                set
            });

        let mut dictionaries = HashMap::new();
        for (locale, raw_json) in EMBEDDED_LOCALES {
            if let Ok(map) = parse_flat_json_map(raw_json) {
                let normalized_key = normalize_locale(locale);
                dictionaries.entry(normalized_key).or_insert(map);
            }
        }

        Self {
            default_locale,
            supported,
            dictionaries,
        }
    }

    pub(super) fn default_locale(&self) -> &str {
        &self.default_locale
    }

    pub(super) fn normalize_or_default(&self, locale: &str) -> String {
        let normalized = normalize_locale(locale);
        if self.supported.contains(&normalized) {
            return normalized;
        }
        normalize_locale(&self.default_locale)
    }

    fn translate(&self, locale: &str, key: &str) -> String {
        let normalized = self.normalize_or_default(locale);

        if let Some(text) = self
            .dictionaries
            .get(&normalized)
            .and_then(|map| map.get(key))
            .cloned()
        {
            return text;
        }

        let default = normalize_locale(&self.default_locale);
        self.dictionaries
            .get(&default)
            .and_then(|map| map.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
}

fn parse_flat_json_map(input: &str) -> GtcResult<HashMap<String, String>> {
    let value: Value = serde_json::from_str(input)
        .map_err(|err| GtcError::json("failed to parse i18n dictionary", err))?;
    let obj = value
        .as_object()
        .ok_or_else(|| GtcError::invalid_data("i18n dictionary", "JSON root must be an object"))?;

    let mut map = HashMap::with_capacity(obj.len());
    for (k, v) in obj {
        let s = v.as_str().ok_or_else(|| {
            GtcError::invalid_data(
                "i18n dictionary",
                format!("translation value for '{k}' must be a string"),
            )
        })?;
        map.insert(k.clone(), s.to_string());
    }
    Ok(map)
}
