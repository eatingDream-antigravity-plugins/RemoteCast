use std::env;
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhCn,
}

impl Lang {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }
}

/// Parses a single locale tag (`zh_CN.UTF-8`, `en-US`, `C`, etc.).
/// Returns `None` for empty or neutral POSIX locales (`C`, `C.UTF-8`, `POSIX`, `UTF-8`)
/// so resolution continues down the fallback chain.
pub fn parse_locale_tag(raw: &str) -> Option<Lang> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '(' || c == ')' || c == ',');
    if trimmed.is_empty() {
        return None;
    }
    let base = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .split('@')
        .next()
        .unwrap_or(trimmed)
        .trim();
    if base.is_empty() {
        return None;
    }
    let low = base.to_ascii_lowercase().replace('_', "-");
    if matches!(low.as_str(), "c" | "posix" | "utf-8" | "utf8") {
        return None;
    }
    if low == "zh" || low.starts_with("zh-") || low.contains("-hans") || low.contains("-hant") {
        Some(Lang::ZhCn)
    } else {
        Some(Lang::En)
    }
}

/// Pure, deterministic language resolution following:
/// `CHROME_STREAM_LANG` -> `LC_ALL` -> `LC_MESSAGES` -> `LANG` -> macOS `AppleLanguages` -> `en`.
pub fn resolve_lang_chain(
    chrome_stream_lang: Option<&str>,
    lc_all: Option<&str>,
    lc_messages: Option<&str>,
    lang: Option<&str>,
    apple_languages: Option<&str>,
) -> Lang {
    for candidate in [chrome_stream_lang, lc_all, lc_messages, lang]
        .into_iter()
        .flatten()
    {
        if let Some(resolved) = parse_locale_tag(candidate) {
            return resolved;
        }
    }
    if let Some(raw_apple) = apple_languages {
        for line in raw_apple.lines() {
            if let Some(resolved) = parse_locale_tag(line) {
                return resolved;
            }
        }
    }
    Lang::En
}

fn cached_macos_apple_languages() -> Option<&'static str> {
    static APPLE_LANGS: OnceLock<Option<String>> = OnceLock::new();
    APPLE_LANGS
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                let out = Command::new("/usr/bin/defaults")
                    .args(["read", "-g", "AppleLanguages"])
                    .output()
                    .ok()?;
                if !out.status.success() {
                    return None;
                }
                Some(String::from_utf8_lossy(&out.stdout).into_owned())
            }
            #[cfg(not(target_os = "macos"))]
            {
                None
            }
        })
        .as_deref()
}

pub fn current_lang() -> Lang {
    let csl = env::var("CHROME_STREAM_LANG").ok();
    let lc_all = env::var("LC_ALL").ok();
    let lc_msg = env::var("LC_MESSAGES").ok();
    let lang = env::var("LANG").ok();

    for v in [&csl, &lc_all, &lc_msg, &lang] {
        if let Some(s) = v.as_deref() {
            if let Some(resolved) = parse_locale_tag(s) {
                return resolved;
            }
        }
    }
    resolve_lang_chain(None, None, None, None, cached_macos_apple_languages())
}

pub fn chrome_not_found_error(lang: Lang) -> &'static str {
    match lang {
        Lang::En => {
            "Chrome remote debugging was not detected. Open Google Chrome and enable remote debugging at chrome://inspect/#remote-debugging (or launch a debug profile with --remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug)."
        }
        Lang::ZhCn => {
            "未检测到本机 Chrome 远程调试。请打开 Google Chrome 并在 chrome://inspect/#remote-debugging 开启远程调试（或使用 --remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug 启动独立调试实例）。"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_language_priority_and_defaults_non_zh_to_en() {
        assert_eq!(
            resolve_lang_chain(Some("zh-CN"), Some("en_US.UTF-8"), None, None, None),
            Lang::ZhCn
        );
        assert_eq!(
            resolve_lang_chain(Some("en"), Some("zh_CN.UTF-8"), None, None, None),
            Lang::En
        );
        assert_eq!(
            resolve_lang_chain(None, Some("zh_TW.UTF-8"), None, Some("en_US.UTF-8"), None),
            Lang::ZhCn
        );
        assert_eq!(
            resolve_lang_chain(None, Some("C"), Some("POSIX"), Some("zh_CN.UTF-8"), None),
            Lang::ZhCn
        );
        assert_eq!(
            resolve_lang_chain(None, None, None, Some("ja_JP.UTF-8"), Some("(\"zh-Hans-CN\")")),
            Lang::En
        );
        assert_eq!(
            resolve_lang_chain(
                None,
                Some("C.UTF-8"),
                None,
                None,
                Some("(\n    \"zh-Hans-CN\",\n    \"en-US\"\n)"),
            ),
            Lang::ZhCn
        );
        assert_eq!(resolve_lang_chain(None, None, None, None, None), Lang::En);
    }
}
