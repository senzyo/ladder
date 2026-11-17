use std::sync::{
    OnceLock,
    atomic::{AtomicU8, Ordering},
};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Language {
    #[serde(rename = "zh-Hans")]
    ZhHans = 0,
    #[serde(rename = "ru-RU")]
    RuRu = 2,
    #[serde(rename = "en-US", other)]
    EnUs = 1,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::ZhHans, Self::EnUs, Self::RuRu];

    pub fn tag(self) -> &'static str {
        match self {
            Self::ZhHans => "zh-Hans",
            Self::EnUs => "en-US",
            Self::RuRu => "ru-RU",
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Self::ZhHans => "简体中文",
            Self::EnUs => "English",
            Self::RuRu => "Русский",
        }
    }

    fn from_system_tag(tag: &str) -> Self {
        let tag = tag.replace('_', "-").to_ascii_lowercase();
        let mut parts = tag.split('-');
        match parts.next() {
            Some("en") => Self::EnUs,
            Some("ru") => Self::RuRu,
            Some("zh") => match parts.next() {
                Some("hans" | "cn" | "sg") | None => Self::ZhHans,
                _ => Self::EnUs,
            },
            _ => Self::EnUs,
        }
    }

    pub fn system() -> Self {
        static SYSTEM: OnceLock<Language> = OnceLock::new();
        *SYSTEM.get_or_init(|| {
            let mut count = 0;
            let mut length = 0;
            // Read the display-language preference, not regional date/number formatting.
            let result = unsafe {
                GetUserPreferredUILanguages(
                    MUI_LANGUAGE_NAME,
                    &mut count,
                    std::ptr::null_mut(),
                    &mut length,
                )
            };
            if result == 0 || length == 0 {
                return Self::EnUs;
            }
            let mut buffer = vec![0u16; length as usize];
            let result = unsafe {
                GetUserPreferredUILanguages(
                    MUI_LANGUAGE_NAME,
                    &mut count,
                    buffer.as_mut_ptr(),
                    &mut length,
                )
            };
            if result == 0 {
                return Self::EnUs;
            }
            let first = buffer.split(|value| *value == 0).next().unwrap_or_default();
            Self::from_system_tag(&String::from_utf16_lossy(first))
        })
    }

    pub fn resolve(saved: Option<Self>) -> Self {
        saved.unwrap_or_else(Self::system)
    }
}

// launch() initializes this before producing any user-facing messages.
// Keeping the source language until then also makes library tests independent of Windows settings.
static CURRENT: AtomicU8 = AtomicU8::new(Language::ZhHans as u8);

pub fn set_current(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn current() -> Language {
    #[cfg(test)]
    if let Some(language) = TEST_LANGUAGE.with(|value| value.get()) {
        return language;
    }
    match CURRENT.load(Ordering::Relaxed) {
        0 => Language::ZhHans,
        2 => Language::RuRu,
        _ => Language::EnUs,
    }
}

macro_rules! message {
    ($name:ident => ($zh:literal, $en:literal, $ru:literal)) => {
        pub(crate) fn $name() -> &'static str {
            match $crate::i18n::current() {
                $crate::i18n::Language::ZhHans => $zh,
                $crate::i18n::Language::EnUs => $en,
                $crate::i18n::Language::RuRu => $ru,
            }
        }
    };
    ($name:ident ($($arg:ident: $ty:ty),* $(,)?) => ($zh:literal, $en:literal, $ru:literal)) => {
        pub(crate) fn $name($($arg: $ty),*) -> String {
            match $crate::i18n::current() {
                $crate::i18n::Language::ZhHans => format!($zh),
                $crate::i18n::Language::EnUs => format!($en),
                $crate::i18n::Language::RuRu => format!($ru),
            }
        }
    };
}

macro_rules! messages {
    ($($name:ident $(($($arg:ident: $ty:ty),* $(,)?))? => ($zh:literal, $en:literal, $ru:literal);)*) => {
        $(
            const _: () = assert!(!$zh.is_empty() && !$en.is_empty() && !$ru.is_empty());
            $crate::i18n::message!($name $(($($arg: $ty),*))? => ($zh, $en, $ru));
        )*
    };
}

pub(crate) use message;

#[path = "i18n/app.rs"]
pub mod app;
#[path = "i18n/runtime.rs"]
pub mod runtime;
#[path = "i18n/system.rs"]
pub mod system;
#[path = "i18n/ui.rs"]
pub mod ui;
#[path = "i18n/updates.rs"]
pub mod updates;

#[cfg(test)]
thread_local! {
    static TEST_LANGUAGE: std::cell::Cell<Option<Language>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_language_matching_and_fallback() {
        for tag in ["zh", "zh-CN", "zh-SG", "zh-Hans", "zh-Hans-CN", "ZH_hans"] {
            assert_eq!(Language::from_system_tag(tag), Language::ZhHans, "{tag}");
        }
        for tag in [
            "en", "en-US", "en-GB", "fr-FR", "ja-JP", "zh-TW", "zh-Hant", "zh-HK", "",
        ] {
            assert_eq!(Language::from_system_tag(tag), Language::EnUs, "{tag}");
        }
        for tag in ["ru", "ru-RU", "ru-BY", "RU_ru"] {
            assert_eq!(Language::from_system_tag(tag), Language::RuRu, "{tag}");
        }
    }

    #[test]
    fn saved_language_tags_round_trip_and_unknown_tags_use_english() {
        for language in Language::ALL {
            let json = serde_json::to_string(&language).unwrap();
            assert_eq!(json, format!("\"{}\"", language.tag()));
            assert_eq!(serde_json::from_str::<Language>(&json).unwrap(), language);
            assert_eq!(Language::resolve(Some(language)), language);
        }
        assert_eq!(
            serde_json::from_str::<Language>("\"fr-FR\"").unwrap(),
            Language::EnUs
        );
        assert_eq!(Language::resolve(None), Language::system());
    }

    messages! {
        sample => ("设置", "Settings", "Настройки");
        interpolated(value: &str, progress: f64) => ("{value}: {progress:.1}%", "{value}: {progress:.1}%", "{value}: {progress:.1}%");
    }

    #[test]
    fn messages_render_in_each_language_without_reinterpreting_values() {
        for (language, expected) in [
            (Language::ZhHans, "设置"),
            (Language::EnUs, "Settings"),
            (Language::RuRu, "Настройки"),
        ] {
            TEST_LANGUAGE.with(|value| value.set(Some(language)));
            assert_eq!(sample(), expected);
            assert_eq!(
                interpolated("{version} / 中文 / Русский", 12.5),
                "{version} / 中文 / Русский: 12.5%"
            );
        }
        TEST_LANGUAGE.with(|value| value.set(None));
    }
}
