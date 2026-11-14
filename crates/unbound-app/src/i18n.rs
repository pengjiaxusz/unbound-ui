//! 国际化、区域格式设置与 RTL 书写方向支持（S5.5）。

use rustc_hash::FxHashMap;

/// 当前激活区域的布局阅读方向。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LayoutDirection {
    /// 从左到右（例如 `en-US`、`zh-CN`）。
    #[default]
    Ltr,
    /// 从右到左（例如 `ar-SA`、`he-IL`）。
    Rtl,
}

/// 轻量级消息资源包与区域感知格式化器。
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
    /// 创建初始化为 `active_locale`（同时作为回退语言）的资源包。
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

    /// 为 `locale` 注册一组静态或动态 `(key, template)` 词条。
    pub fn register_locale(&mut self, locale: impl Into<String>, entries: &[(&str, &str)]) {
        let map = self.tables.entry(locale.into()).or_default();
        for &(k, v) in entries {
            map.insert(k.to_owned(), v.to_owned());
        }
    }

    /// 在运行时切换当前激活区域。
    pub fn set_locale(&mut self, locale: impl Into<String>) {
        self.active = locale.into();
    }

    /// 设置当激活区域缺失键时使用的回退区域。
    pub fn set_fallback(&mut self, locale: impl Into<String>) {
        self.fallback = locale.into();
    }

    /// 当前激活的区域标签（例如 `"zh-CN"`）。
    #[must_use]
    pub fn locale(&self) -> &str {
        &self.active
    }

    /// 覆盖 RTL 检测（`Some(true)` 强制 RTL，`None` 由区域自动推导）。
    pub fn set_rtl_override(&mut self, rtl: Option<bool>) {
        self.rtl_override = rtl;
    }

    /// 当前激活区域是否按从右到左布局。
    #[must_use]
    pub fn is_rtl(&self) -> bool {
        if let Some(forced) = self.rtl_override {
            return forced;
        }
        let lower = self.active.to_ascii_lowercase();
        let lang = lower.split(['-', '_']).next().unwrap_or("");
        matches!(lang, "ar" | "he" | "fa" | "ur" | "iw")
    }

    /// 当前激活的 [`LayoutDirection`]。
    #[must_use]
    pub fn direction(&self) -> LayoutDirection {
        if self.is_rtl() {
            LayoutDirection::Rtl
        } else {
            LayoutDirection::Ltr
        }
    }

    /// 按 `key` 查找原始消息模板，依次回退至 `fallback` 与 `key` 本身。
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

    /// 查找 `key` 并使用 `args` 替换 `{name}` 占位符。
    #[must_use]
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut out = self.t(key).to_owned();
        for &(name, value) in args {
            let token = format!("{{{name}}}");
            out = out.replace(&token, value);
        }
        out
    }

    /// 按区域习惯的千分位与小数点分隔符格式化数字。
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

    /// 按当前激活区域格式化日历日期。
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
        assert_eq!(bundle.t("save"), "Save"); // 回退到 en-US
        assert_eq!(bundle.format_date(2026, 11, 13), "2026年11月13日");

        bundle.set_locale("ar-SA");
        assert!(bundle.is_rtl());
        assert_eq!(bundle.direction(), LayoutDirection::Rtl);
        assert_eq!(bundle.format_date(2026, 11, 13), "13/11/2026");
    }
}
