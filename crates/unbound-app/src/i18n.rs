//! Internationalization, locale formatting, and RTL directionality (S5.5).

use rustc_hash::FxHashMap;

/// Layout reading direction for the active locale.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LayoutDirection {
    /// Left-to-right (e.g. `en-US`, `zh-CN`).
    #[default]
    Ltr,
    /// Right-to-left (e.g. `ar-SA`, `he-IL`).
    Rtl,
}

/// Lightweight message bundle and locale-aware formatter.
#[derive(Clone, Debug)]
pub struct I18nBundle {
    active: String,
    fallback: String,
    rtl_override: Option<bool>,
    tables: FxHashMap<String, FxHashMap<String, String>>,
}

impl Default for I18nBundle {
    fn default() -> Self {
        Self::new("en-US")
    }
}

impl I18nBundle {
    /// Create a bundle initialized to `active_locale` (also used as fallback).
    #[must_use]
    pub fn new(active_locale: impl Into<String>) -> Self {
        let locale = active_locale.into();
        Self {
            active: locale.clone(),
            fallback: locale,
            rtl_override: None,
            tables: FxHashMap::default(),
        }
    }

    /// Register a static or dynamic slice of `(key, template)` entries for `locale`.
    pub fn register_locale(&mut self, locale: impl Into<String>, entries: &[(&str, &str)]) {
        let map = self.tables.entry(locale.into()).or_default();
        for &(k, v) in entries {
            map.insert(k.to_owned(), v.to_owned());
        }
    }

    /// Switch the active locale at runtime.
    pub fn set_locale(&mut self, locale: impl Into<String>) {
        self.active = locale.into();
    }

    /// Set the fallback locale used when a key is missing in the active locale.
    pub fn set_fallback(&mut self, locale: impl Into<String>) {
        self.fallback = locale.into();
    }

    /// Currently active locale tag (e.g. `"zh-CN"`).
    #[must_use]
    pub fn locale(&self) -> &str {
        &self.active
    }

    /// Override RTL detection (`Some(true)` forces RTL, `None` derives from locale).
    pub fn set_rtl_override(&mut self, rtl: Option<bool>) {
        self.rtl_override = rtl;
    }

    /// Whether the active locale lays out right-to-left.
    #[must_use]
    pub fn is_rtl(&self) -> bool {
        if let Some(forced) = self.rtl_override {
            return forced;
        }
        let lower = self.active.to_ascii_lowercase();
        let lang = lower.split(['-', '_']).next().unwrap_or("");
        matches!(lang, "ar" | "he" | "fa" | "ur" | "iw")
    }

    /// Active [`LayoutDirection`].
    #[must_use]
    pub fn direction(&self) -> LayoutDirection {
        if self.is_rtl() {
            LayoutDirection::Rtl
        } else {
            LayoutDirection::Ltr
        }
    }

    /// Look up a raw message template by `key`, falling back to `fallback` then `key`.
    #[must_use]
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        if let Some(val) = self.tables.get(&self.active).and_then(|m| m.get(key)) {
            return val.as_str();
        }
        if let Some(val) = self.tables.get(&self.fallback).and_then(|m| m.get(key)) {
            return val.as_str();
        }
        key
    }

    /// Look up `key` and substitute `{name}` placeholders from `args`.
    #[must_use]
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut out = self.t(key).to_owned();
        for &(name, value) in args {
            let token = format!("{{{name}}}");
            out = out.replace(&token, value);
        }
        out
    }

    /// Format a number with locale-appropriate grouping and decimal separators.
    #[must_use]
    pub fn format_number(&self, value: f64, decimals: usize) -> String {
        let neg = value.is_sign_negative() && value != 0.0;
        let abs = value.abs();
        let raw = format!("{abs:.decimals$}");
        let (int_part, frac_part) = match raw.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (raw.as_str(), None),
        };
        let lower = self.active.to_ascii_lowercase();
        let (thousands_sep, decimal_sep) = if lower.starts_with("de") || lower.starts_with("fr") {
            ('.', ',')
        } else {
            (',', '.')
        };
        let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3 + 8);
        if neg {
            grouped.push('-');
        }
        for (idx, ch) in int_part.chars().rev().enumerate() {
            if idx > 0 && idx % 3 == 0 {
                grouped.push(thousands_sep);
            }
            grouped.push(ch);
        }
        let rev_start = usize::from(neg);
        let digits_rev: String = grouped[rev_start..].chars().rev().collect();
        grouped.replace_range(rev_start.., &digits_rev);
        if let Some(frac) = frac_part {
            if !frac.is_empty() {
                grouped.push(decimal_sep);
                grouped.push_str(frac);
            }
        }
        grouped
    }

    /// Format a calendar date according to the active locale.
    #[must_use]
    pub fn format_date(&self, year: i32, month: u8, day: u8) -> String {
        let lower = self.active.to_ascii_lowercase();
        if lower.starts_with("zh") || lower.starts_with("ja") {
            format!("{year:04}年{month:02}月{day:02}日")
        } else if lower.starts_with("en-us") || lower == "en" {
            format!("{month:02}/{day:02}/{year:04}")
        } else {
            format!("{day:02}/{month:02}/{year:04}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i18n_switches_locales_interpolates_formats_and_flips_rtl() {
        let mut bundle = I18nBundle::new("en-US");
        bundle.register_locale("en-US", &[("greeting", "Hello, {name}!"), ("save", "Save")]);
        bundle.register_locale("zh-CN", &[("greeting", "你好，{name}！")]);

        assert_eq!(bundle.direction(), LayoutDirection::Ltr);
        assert_eq!(
            bundle.format("greeting", &[("name", "Alice")]),
            "Hello, Alice!"
        );
        assert_eq!(bundle.format_number(1234567.89, 2), "1,234,567.89");
        assert_eq!(bundle.format_date(2026, 11, 13), "11/13/2026");

        bundle.set_locale("zh-CN");
        assert_eq!(
            bundle.format("greeting", &[("name", "Alice")]),
            "你好，Alice！"
        );
        assert_eq!(bundle.t("save"), "Save"); // fallback to en-US
        assert_eq!(bundle.format_date(2026, 11, 13), "2026年11月13日");

        bundle.set_locale("ar-SA");
        assert!(bundle.is_rtl());
        assert_eq!(bundle.direction(), LayoutDirection::Rtl);
        assert_eq!(bundle.format_date(2026, 11, 13), "13/11/2026");
    }
}
