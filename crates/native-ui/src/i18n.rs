//! Presentation-only notification translation; shared catalogs with native clients.
use std::{collections::BTreeMap, io::Read, path::Path, sync::OnceLock};
type Catalogs = BTreeMap<String, BTreeMap<String, String>>;
fn catalogs() -> &'static Catalogs {
    static CATALOGS: OnceLock<Catalogs> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        super::LOCALES
            .iter()
            .map(|(code, json)| {
                (
                    code.to_string(),
                    serde_json::from_str(json).expect("bundled locale"),
                )
            })
            .collect()
    })
}
fn code(value: &str) -> Option<String> {
    let value = value.split(['.', '@', '-', '_']).next()?.to_lowercase();
    catalogs().contains_key(&value).then_some(value)
}
fn system_language(env: &BTreeMap<String, String>) -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| env.get(*key).filter(|s| !s.is_empty()))
        .map(String::as_str)
        .unwrap_or("en");
    if matches!(locale, "C" | "POSIX" | "C.UTF-8" | "C.utf8") {
        return "en".into();
    }
    env.get("LANGUAGE")
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .unwrap_or(locale)
        .split(':')
        .find_map(code)
        .unwrap_or_else(|| "en".into())
}
fn language(profile: &Path) -> String {
    let preference = (|| {
        let mut text = String::new();
        std::fs::File::open(profile.join("ui-preferences.json"))
            .ok()?
            .take(4097)
            .read_to_string(&mut text)
            .ok()?;
        if text.len() > 4096 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        let choice = value.get("language")?.as_str()?;
        catalogs().contains_key(choice).then(|| choice.to_owned())
    })();
    preference.unwrap_or_else(|| system_language(&std::env::vars().collect()))
}
fn translate(text: &str, language: &str) -> String {
    let all = catalogs();
    let english = &all["en"];
    let selected = all.get(language).unwrap_or(english);
    if let Some((key, _)) = english
        .iter()
        .find(|(key, value)| key.starts_with("notice.") && value.as_str() == text)
    {
        return selected.get(key).unwrap_or(&english[key]).clone();
    }
    if let Some(count) = text
        .strip_suffix(" device(s)")
        .and_then(|s| s.parse::<u64>().ok())
    {
        let one = count == 1 || (language == "fr" && count == 0);
        let key = if one {
            "notice.devices.one"
        } else {
            "notice.devices.other"
        };
        return selected
            .get(key)
            .unwrap_or(&english[key])
            .replace("{count}", &count.to_string());
    }
    // OS/network diagnostics and user/device content must never be guessed or rewritten.
    text.into()
}
pub fn notice(profile: &Path, title: &str, body: &str) -> (String, String) {
    let language = language(profile);
    (translate(title, &language), translate(body, &language))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notices_use_shared_catalogs_and_preserve_unknown_diagnostics() {
        for (locale, sent, one, many) in [
            ("en", "Clipboard sent", "1 device", "2 devices"),
            ("nl", "Klembord verzonden", "1 apparaat", "2 apparaten"),
            ("de", "Zwischenablage gesendet", "1 Gerät", "2 Geräte"),
            ("fr", "Presse-papiers envoyé", "1 appareil", "2 appareils"),
        ] {
            assert_eq!(translate("Clipboard sent", locale), sent);
            assert_eq!(translate("1 device(s)", locale), one);
            assert_eq!(translate("2 device(s)", locale), many);
            assert_eq!(
                translate("TLS diagnostic {raw}", locale),
                "TLS diagnostic {raw}"
            );
        }
        assert_eq!(translate("0 device(s)", "fr"), "0 appareil");
    }
    #[test]
    fn delivery_outcomes_have_distinct_localized_notices() {
        for locale in ["en", "nl", "de", "fr"] {
            let titles = [
                "Clipboard transfer failed",
                "Clipboard transfer cancelled",
                "Clipboard transfer replaced by a newer copy",
                "Clipboard transfer not confirmed",
            ]
            .map(|title| translate(title, locale));
            assert_eq!(
                titles
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                4
            );
            if locale != "en" {
                assert!(titles.iter().all(|title| !title.starts_with("Clipboard")));
            }
        }
    }
    #[test]
    fn system_locale_precedence_and_fallback_are_predictable() {
        let mut env = BTreeMap::from([("LANG".into(), "nl_NL.UTF-8".into())]);
        assert_eq!(system_language(&env), "nl");
        env.insert("LC_MESSAGES".into(), "de-DE".into());
        assert_eq!(system_language(&env), "de");
        env.insert("LANGUAGE".into(), "xx:fr_FR:en".into());
        assert_eq!(system_language(&env), "fr");
        env.insert("LC_ALL".into(), "C".into());
        assert_eq!(system_language(&env), "en");
    }
}
