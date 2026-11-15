//! 国际化、区域格式设置与 RTL 书写方向支持（S5.5）。
//!
//! 提供引擎级原生多语言与本地化支持：
//! - [`Locale`]：标准常用语言预设（默认 `English` `en-US` 与 `简体中文` `zh-CN` 等）。
//! - [`I18nBundle`]：零外部重型依赖的轻量级词条资源包，支持回退链、命名空间、参数插值（`{name}` / `{{name}}`）、单复数选择、数字千分位与日期本地化格式，以及 RTL 布局方向推导。
//! - 全局与上下文级绑定：支持通过 [`global_locale`] / [`set_global_locale`] 或 `AppContext::i18n` 在声明式视图与自举外壳中无缝读取当前激活语言。

use std::sync::atomic::{AtomicU8, Ordering};

use rustc_hash::FxHashMap;

/// 内置标准语言区域预设（默认 `English`）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Locale {
    /// 英语（美国，`en-US`）—— 默认语言。
    #[default]
    EnUs = 0,
    /// 简体中文（中国大陆，`zh-CN`）。
    ZhCn = 1,
}

static GLOBAL_LOCALE: AtomicU8 = AtomicU8::new(0);

/// 获取全局当前激活的标准语言区域。
#[must_use]
pub fn global_locale() -> Locale {
    Locale::from_index(GLOBAL_LOCALE.load(Ordering::Relaxed) as usize)
}

/// 设置全局当前激活的标准语言区域。
pub fn set_global_locale(locale: Locale) {
    GLOBAL_LOCALE.store(locale as u8, Ordering::Relaxed);
}

impl Locale {
    /// 全部内置预设语言列表（顺序：`English`、`简体中文`）。
    pub const ALL: [Self; 2] = [Self::EnUs, Self::ZhCn];

    /// 按索引还原 [`Locale`]，超出范围时回退至默认 [`Self::EnUs`]。
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        match index {
            1 => Self::ZhCn,
            _ => Self::EnUs,
        }
    }

    /// 预设语言在 [`Self::ALL`] 中的稳定索引。
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// 标准 BCP-47 语言标签（如 `"en-US"`、`"zh-CN"`）。
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }

    /// 语言自称显示名（用于标题栏语言下拉选择器与设置项）。
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::EnUs => "English",
            Self::ZhCn => "简体中文",
        }
    }

    /// 紧凑短标签（用于窄标题栏胶囊显示）。
    #[must_use]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::EnUs => "EN",
            Self::ZhCn => "中文",
        }
    }

    /// 从任意 BCP-47 标签字符串解析内置 [`Locale`]（不区分大小写，如 `"zh"`、`"zh-CN"`、`"en"`、`"en-US"`）。
    #[must_use]
    pub fn from_tag(tag: &str) -> Self {
        let lower = tag.trim().to_ascii_lowercase();
        if lower.starts_with("zh") {
            Self::ZhCn
        } else {
            Self::EnUs
        }
    }

    /// 是否为简体中文。
    #[must_use]
    pub const fn is_zh(self) -> bool {
        matches!(self, Self::ZhCn)
    }

    /// 在两种内置语言之间快速切换。
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::EnUs => Self::ZhCn,
            Self::ZhCn => Self::EnUs,
        }
    }

    /// 双语内联选择辅助方法：英文返回 `en`，简体中文返回 `zh`。
    #[must_use]
    pub const fn pick<'a>(self, en: &'a str, zh: &'a str) -> &'a str {
        match self {
            Self::EnUs => en,
            Self::ZhCn => zh,
        }
    }
}

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
        Self::new(Locale::EnUs.tag())
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

    /// 使用内置 [`Locale`] 创建资源包（回退语言固定为 `en-US`）。
    #[must_use]
    pub fn with_locale(locale: Locale) -> Self {
        let mut bundle = Self::new(locale.tag());
        bundle.set_fallback(Locale::EnUs.tag());
        bundle
    }

    /// 为 `locale` 注册一组静态或动态 `(key, template)` 词条。
    pub fn register_locale(&mut self, locale: impl Into<String>, entries: &[(&str, &str)]) {
        let map = self.tables.entry(locale.into()).or_default();
        for &(k, v) in entries {
            map.insert(k.to_owned(), v.to_owned());
        }
    }

    /// 同时注册 `en-US` 与 `zh-CN` 双语词条表 `(key, en_text, zh_text)`。
    pub fn register_bilingual(&mut self, entries: &[(&str, &str, &str)]) {
        {
            let en_map = self
                .tables
                .entry(Locale::EnUs.tag().to_owned())
                .or_default();
            for &(k, en, _) in entries {
                en_map.insert(k.to_owned(), en.to_owned());
            }
        }
        {
            let zh_map = self
                .tables
                .entry(Locale::ZhCn.tag().to_owned())
                .or_default();
            for &(k, _, zh) in entries {
                zh_map.insert(k.to_owned(), zh.to_owned());
            }
        }
    }

    /// 在运行时切换当前激活区域，并同步更新全局 [`Locale`]。
    pub fn set_locale(&mut self, locale: impl Into<String>) {
        let tag = locale.into();
        set_global_locale(Locale::from_tag(&tag));
        self.active = tag;
    }

    /// 通过强类型 [`Locale`] 枚举切换当前激活区域。
    pub fn set_locale_enum(&mut self, locale: Locale) {
        set_global_locale(locale);
        self.active = locale.tag().to_owned();
    }

    /// 获取当前激活区域对应的 [`Locale`] 枚举。
    #[must_use]
    pub fn active_locale_enum(&self) -> Locale {
        Locale::from_tag(&self.active)
    }

    /// 设置当激活区域缺失键时使用的回退区域。
    pub fn set_fallback(&mut self, locale: impl Into<String>) {
        self.fallback = locale.into();
    }

    /// 获取当前回退区域标签。
    #[must_use]
    pub fn fallback_locale(&self) -> &str {
        &self.fallback
    }

    /// 当前激活的区域标签（例如 `"en-US"`、`"zh-CN"`）。
    #[must_use]
    pub fn locale(&self) -> &str {
        &self.active
    }

    /// 返回当前已注册的所有区域标签列表（按字典序排序以确保确定性）。
    #[must_use]
    pub fn available_locales(&self) -> Vec<String> {
        let mut locales: Vec<String> = self.tables.keys().cloned().collect();
        locales.sort();
        locales
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

    /// 检查指定 `key` 在当前激活区域或回退区域中是否已注册。
    #[must_use]
    pub fn has_key(&self, key: &str) -> bool {
        self.tables
            .get(&self.active)
            .is_some_and(|m| m.contains_key(key))
            || self
                .tables
                .get(&self.fallback)
                .is_some_and(|m| m.contains_key(key))
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

    /// 查找 `key` 并使用 `args` 替换 `{name}` 或 `{{name}}`（i18next 兼容）占位符。
    #[must_use]
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut out = self.t(key).to_owned();
        for &(name, value) in args {
            let double_token = format!("{{{{{name}}}}}");
            out = out.replace(&double_token, value);
            let single_token = format!("{{{name}}}");
            out = out.replace(&single_token, value);
        }
        out
    }

    /// 按数量 `count` 进行单复数格式化：
    /// 优先查找 `{key}.zero`（当 `count == 0`）、`{key}.one`（当 `count == 1`）、`{key}.other`，
    /// 若未注册子键则回退到 `key` 本身，并自动注入 `{count}` 占位符。
    #[must_use]
    pub fn format_plural(&self, key: &str, count: usize) -> String {
        let suffix = match count {
            0 => "zero",
            1 => "one",
            _ => "other",
        };
        let candidate = format!("{key}.{suffix}");
        let other_candidate = format!("{key}.other");
        let count_str = count.to_string();
        if self.has_key(&candidate) {
            self.format(&candidate, &[("count", &count_str)])
        } else if self.has_key(&other_candidate) {
            self.format(&other_candidate, &[("count", &count_str)])
        } else {
            self.format(key, &[("count", &count_str)])
        }
    }

    /// 按命名空间与键路径查找词条（例如 `t_ns("settings", "appearance.theme")` 等价于查找 `"settings:appearance.theme"` 或 `"settings.appearance.theme"`）。
    #[must_use]
    pub fn t_ns<'a>(&'a self, namespace: &str, key: &'a str) -> &'a str {
        let colon_key = format!("{namespace}:{key}");
        if let Some(val) = self
            .tables
            .get(&self.active)
            .and_then(|m| m.get(&colon_key))
        {
            return val.as_str();
        }
        let dot_key = format!("{namespace}.{key}");
        if let Some(val) = self.tables.get(&self.active).and_then(|m| m.get(&dot_key)) {
            return val.as_str();
        }
        if let Some(val) = self
            .tables
            .get(&self.fallback)
            .and_then(|m| m.get(&colon_key))
        {
            return val.as_str();
        }
        if let Some(val) = self
            .tables
            .get(&self.fallback)
            .and_then(|m| m.get(&dot_key))
        {
            return val.as_str();
        }
        self.t(key)
    }

    /// 返回指定区域已注册的所有键数量（供 `xtask audit` 校验多语言键完整性）。
    #[must_use]
    pub fn key_count(&self, locale: &str) -> usize {
        self.tables.get(locale).map_or(0, FxHashMap::len)
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

    #[test]
    fn bilingual_registration_and_pluralization_work_cleanly() {
        let mut bundle = I18nBundle::with_locale(Locale::EnUs);
        bundle.register_bilingual(&[
            ("items.zero", "No items selected", "未选中任何项"),
            ("items.one", "1 item selected", "已选中 1 项"),
            ("items.other", "{count} items selected", "已选中 {count} 项"),
            ("nav:home", "Home", "首页"),
        ]);

        assert_eq!(bundle.format_plural("items", 0), "No items selected");
        assert_eq!(bundle.format_plural("items", 1), "1 item selected");
        assert_eq!(bundle.format_plural("items", 5), "5 items selected");
        assert_eq!(bundle.t_ns("nav", "home"), "Home");

        bundle.set_locale_enum(Locale::ZhCn);
        assert_eq!(bundle.active_locale_enum(), Locale::ZhCn);
        assert_eq!(bundle.format_plural("items", 0), "未选中任何项");
        assert_eq!(bundle.format_plural("items", 5), "已选中 5 项");
        assert_eq!(bundle.t_ns("nav", "home"), "首页");
    }
}
