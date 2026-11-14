//! 零分配友好的全量汉字（全拼 + 6 套双拼 + 全拼/双拼首字母 + 多音字）检索引擎。
//!
//! 面向 `dunting`（中文资产库、标签、文件夹检索）、`chahu-render-debugger`（命令面板、
//! 进程名、捕获历史、设置项搜索）与 `Showcase` 全局检索：
//! * **全字库覆盖**：覆盖 Unicode CJK 基本汉字（`U+4E00..=U+9FFF`，共 20,992 字）、
//!   扩展 A 区汉字（`U+3400..=U+4DBF`，共 6,592 字）、`〇`（`U+3007`）及 200+ 高频多音字；
//! * **全拼匹配（Full Pinyin）**：如 `"caizhi"` → `"材质球"`、`"zhuoseqi"` → `"着色器"`；
//! * **双拼匹配（Shuangpin）**：内置 6 套主流双拼方案，**默认开启小鹤双拼（[`ShuangpinScheme::Xiaohe`]）**
//!   （如小鹤双拼 `"voseqi"` / `"veseqi"` → `"着色器"`，`"cdviqq"` → `"材质球"`）；
//! * **全拼与双拼首字母缩写（Initials）**：同时支持全拼首字母（如 `"zsq"` → `"着色器"`、`"cmm"` → `"重命名"`）
//!   与当前双拼方案首字母（如小鹤双拼 `"vsq"` → `"着色器"`、`"imm"` → `"重命名"`）；
//! * **运行时动态调整**：支持通过 [`set_global_shuangpin_scheme`] / [`set_global_pinyin_config`]
//!   全局无锁原子切换，或通过 [`PinyinMatcher::with_config`] / [`PinyinMatcher::set_scheme`] 实例级调整。

use core::sync::atomic::{AtomicU32, Ordering};

use crate::pinyin_data::{
    CJK_EXT_A_SYLLABLES, CJK_POLYPHONES, CJK_UNIFIED_SYLLABLES, INVALID_SYLLABLE,
    SINGLE_SYLLABLE_READINGS, SYLLABLE_STRS,
};

/// 内置双拼键位方案（默认开启小鹤双拼 [`Self::Xiaohe`]）。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShuangpinScheme {
    /// 小鹤双拼（默认）：`zh=v, ch=i, sh=u`，零声母以首字母+韵母键编码（`ang=ah, eng=eg`）。
    #[default]
    Xiaohe = 0,
    /// 自然码双拼：`zh=v, ch=i, sh=u`，零声母以首字母+韵母键编码。
    Natural = 1,
    /// 微软双拼：`zh=v, ch=i, sh=u`，零声母固定以 `o` 引导，`üe=v`。
    Microsoft = 2,
    /// 搜狗双拼：`zh=v, ch=i, sh=u`，零声母固定以 `o` 引导，`üe=v`。
    Sogou = 3,
    /// 紫光双拼（华宇双拼）：`zh=u, ch=a, sh=i`，零声母固定以 `o` 引导。
    Ziguang = 4,
    /// 智能 ABC 双拼：`zh=a, ch=e, sh=v`，零声母固定以 `o` 引导。
    SmartAbc = 5,
}

impl ShuangpinScheme {
    /// 全部内置双拼方案列表。
    pub const ALL: [Self; 6] = [
        Self::Xiaohe,
        Self::Natural,
        Self::Microsoft,
        Self::Sogou,
        Self::Ziguang,
        Self::SmartAbc,
    ];

    /// 方案在 [`Self::ALL`] 中的索引。
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// 从索引还原双拼方案（越界时回退到默认 [`Self::Xiaohe`]）。
    #[must_use]
    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Xiaohe,
            1 => Self::Natural,
            2 => Self::Microsoft,
            3 => Self::Sogou,
            4 => Self::Ziguang,
            5 => Self::SmartAbc,
            _ => Self::Xiaohe,
        }
    }

    /// 方案中文与英文组合展示名称。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Xiaohe => "小鹤双拼 (Xiaohe)",
            Self::Natural => "自然码 (Natural)",
            Self::Microsoft => "微软双拼 (Microsoft)",
            Self::Sogou => "搜狗双拼 (Sogou)",
            Self::Ziguang => "紫光双拼 (Ziguang)",
            Self::SmartAbc => "智能 ABC (SmartABC)",
        }
    }

    /// 方案紧凑英文短标签（适合分段控制器与状态栏）。
    #[must_use]
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Xiaohe => "Xiaohe",
            Self::Natural => "Natural",
            Self::Microsoft => "Microsoft",
            Self::Sogou => "Sogou",
            Self::Ziguang => "Ziguang",
            Self::SmartAbc => "SmartABC",
        }
    }

    /// 将单个全拼音节（如 `"zhuo"`、`"cai"`、`"ang"`、`"lve"`）编码为当前双拼方案下的 2 字节按键码。
    #[must_use]
    pub fn encode_syllable(self, syllable: &str) -> Option<[u8; 2]> {
        if syllable.is_empty() {
            return None;
        }
        // 1. 零声母音节直接映射
        let zero = match self {
            Self::Xiaohe | Self::Natural => match syllable {
                "a" => Some(*b"aa"),
                "ai" => Some(*b"ai"),
                "an" => Some(*b"an"),
                "ang" => Some(*b"ah"),
                "ao" => Some(*b"ao"),
                "e" => Some(*b"ee"),
                "ei" => Some(*b"ei"),
                "en" => Some(*b"en"),
                "eng" => Some(*b"eg"),
                "er" => Some(*b"er"),
                "o" => Some(*b"oo"),
                "ou" => Some(*b"ou"),
                _ => None,
            },
            Self::Microsoft | Self::Sogou => match syllable {
                "a" => Some(*b"oa"),
                "ai" => Some(*b"ol"),
                "an" => Some(*b"oj"),
                "ang" => Some(*b"oh"),
                "ao" => Some(*b"ok"),
                "e" => Some(*b"oe"),
                "ei" => Some(*b"oz"),
                "en" => Some(*b"of"),
                "eng" => Some(*b"og"),
                "er" => Some(*b"or"),
                "o" => Some(*b"oo"),
                "ou" => Some(*b"ob"),
                _ => None,
            },
            Self::Ziguang => match syllable {
                "a" => Some(*b"oa"),
                "ai" => Some(*b"op"),
                "an" => Some(*b"or"),
                "ang" => Some(*b"os"),
                "ao" => Some(*b"oq"),
                "e" => Some(*b"oe"),
                "ei" => Some(*b"ok"),
                "en" => Some(*b"ow"),
                "eng" => Some(*b"ot"),
                "er" => Some(*b"oj"),
                "o" => Some(*b"oo"),
                "ou" => Some(*b"oz"),
                _ => None,
            },
            Self::SmartAbc => match syllable {
                "a" => Some(*b"oa"),
                "ai" => Some(*b"ol"),
                "an" => Some(*b"oj"),
                "ang" => Some(*b"oh"),
                "ao" => Some(*b"ok"),
                "e" => Some(*b"oe"),
                "ei" => Some(*b"oq"),
                "en" => Some(*b"of"),
                "eng" => Some(*b"og"),
                "er" => Some(*b"or"),
                "o" => Some(*b"oo"),
                "ou" => Some(*b"ob"),
                _ => None,
            },
        };
        if let Some(code) = zero {
            return Some(code);
        }

        // 2. 拆分声母与韵母
        let (ini_key, raw_final) = if let Some(rest) = syllable.strip_prefix("zh") {
            let k = match self {
                Self::Xiaohe | Self::Natural | Self::Microsoft | Self::Sogou => b'v',
                Self::Ziguang => b'u',
                Self::SmartAbc => b'a',
            };
            (k, rest)
        } else if let Some(rest) = syllable.strip_prefix("ch") {
            let k = match self {
                Self::Xiaohe | Self::Natural | Self::Microsoft | Self::Sogou => b'i',
                Self::Ziguang => b'a',
                Self::SmartAbc => b'e',
            };
            (k, rest)
        } else if let Some(rest) = syllable.strip_prefix("sh") {
            let k = match self {
                Self::Xiaohe | Self::Natural | Self::Microsoft | Self::Sogou => b'u',
                Self::Ziguang => b'i',
                Self::SmartAbc => b'v',
            };
            (k, rest)
        } else {
            let first = *syllable.as_bytes().first()?;
            if !first.is_ascii_lowercase() {
                return None;
            }
            (first, &syllable[1..])
        };

        // 归一化 `ue` -> `ve`（`jue`/`que`/`xue`/`yue`/`lue`/`nue` 的韵母均为 `üe`）
        let fin = if raw_final == "ue" { "ve" } else { raw_final };

        let fin_key = match fin {
            "a" => b'a',
            "o" => b'o',
            "e" => b'e',
            "i" => b'i',
            "u" => b'u',
            "v" => b'v',
            "ai" => match self {
                Self::Xiaohe => b'd',
                Self::Ziguang => b'p',
                Self::Natural | Self::Microsoft | Self::Sogou | Self::SmartAbc => b'l',
            },
            "ei" => match self {
                Self::Xiaohe => b'w',
                Self::Ziguang => b'k',
                Self::SmartAbc => b'q',
                Self::Natural | Self::Microsoft | Self::Sogou => b'z',
            },
            "ui" => match self {
                Self::SmartAbc => b'm',
                _ => b'v',
            },
            "ao" => match self {
                Self::Xiaohe => b'c',
                Self::Ziguang => b'q',
                _ => b'k',
            },
            "ou" => match self {
                Self::Xiaohe | Self::Ziguang => b'z',
                _ => b'b',
            },
            "iu" => match self {
                Self::Ziguang => b'j',
                Self::SmartAbc => b'r',
                _ => b'q',
            },
            "ie" => match self {
                Self::Xiaohe => b'p',
                Self::Ziguang => b'd',
                _ => b'x',
            },
            "ve" => match self {
                Self::Microsoft | Self::Sogou => b'v',
                _ => b't',
            },
            "an" => match self {
                Self::Ziguang => b'r',
                _ => b'j',
            },
            "en" => match self {
                Self::Ziguang => b'w',
                _ => b'f',
            },
            "in" => match self {
                Self::Xiaohe => b'b',
                Self::Ziguang => b'y',
                Self::SmartAbc => b'c',
                _ => b'n',
            },
            "un" => match self {
                Self::Xiaohe => b'y',
                Self::Ziguang => b'm',
                Self::SmartAbc => b'n',
                _ => b'p',
            },
            "ang" => match self {
                Self::Ziguang => b's',
                _ => b'h',
            },
            "eng" => match self {
                Self::Ziguang => b't',
                _ => b'g',
            },
            "ing" => match self {
                Self::Xiaohe => b'k',
                _ => b'y',
            },
            "ong" | "iong" => match self {
                Self::Ziguang => b'h',
                _ => b's',
            },
            "ia" | "ua" => match self {
                Self::Xiaohe | Self::Ziguang => b'x',
                Self::SmartAbc => b'd',
                _ => b'w',
            },
            "iao" => match self {
                Self::Xiaohe => b'n',
                Self::Ziguang => b'b',
                Self::SmartAbc => b'z',
                _ => b'c',
            },
            "ian" => match self {
                Self::Ziguang => b'f',
                Self::SmartAbc => b'w',
                _ => b'm',
            },
            "iang" | "uang" => match self {
                Self::Xiaohe => b'l',
                Self::Ziguang => b'g',
                Self::SmartAbc => b't',
                _ => b'd',
            },
            "uo" => b'o',
            "uai" => match self {
                Self::Xiaohe => b'k',
                Self::SmartAbc => b'c',
                _ => b'y',
            },
            "uan" => match self {
                Self::Ziguang => b'l',
                Self::SmartAbc => b'p',
                _ => b'r',
            },
            _ => return None,
        };

        Some([ini_key, fin_key])
    }

    /// 获取音节在当前双拼方案下的首码字符（即双拼首字母，如小鹤双拼 `"zhuo"` -> `'v'`）。
    #[must_use]
    pub fn initial_key(self, syllable: &str) -> Option<char> {
        self.encode_syllable(syllable).map(|pair| pair[0] as char)
    }

    /// 将一段中英文混合文本编码为当前双拼方案下的主读音双拼字符串（用于 UI 预览与调试）。
    #[must_use]
    pub fn encode_text(self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            if let Some(&primary) = PinyinMatcher::readings(ch).first() {
                if let Some(pair) = self.encode_syllable(primary) {
                    out.push(pair[0] as char);
                    out.push(pair[1] as char);
                    continue;
                }
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        }
        out
    }
}

/// 拼音与双拼匹配器的运行时配置。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PinyinConfig {
    /// 当前激活的双拼方案（默认 [`ShuangpinScheme::Xiaohe`] 小鹤双拼）。
    pub shuangpin_scheme: ShuangpinScheme,
    /// 是否启用全拼匹配（默认 `true`）。
    pub enable_full_pinyin: bool,
    /// 是否启用双拼匹配（默认 `true`）。
    pub enable_shuangpin: bool,
    /// 是否启用全拼首字母缩写匹配（默认 `true`）。
    pub enable_full_initials: bool,
    /// 是否启用双拼首字母缩写匹配（默认 `true`）。
    pub enable_shuangpin_initials: bool,
}

impl Default for PinyinConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl PinyinConfig {
    /// 创建默认配置（开启全拼 + 双拼 + 全拼首字母 + 双拼首字母，双拼方案默认为小鹤双拼）。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            shuangpin_scheme: ShuangpinScheme::Xiaohe,
            enable_full_pinyin: true,
            enable_shuangpin: true,
            enable_full_initials: true,
            enable_shuangpin_initials: true,
        }
    }

    /// 指定双拼方案。
    #[must_use]
    pub const fn with_scheme(mut self, scheme: ShuangpinScheme) -> Self {
        self.shuangpin_scheme = scheme;
        self
    }

    /// 开关全拼匹配。
    #[must_use]
    pub const fn with_full_pinyin(mut self, enabled: bool) -> Self {
        self.enable_full_pinyin = enabled;
        self
    }

    /// 开关双拼匹配。
    #[must_use]
    pub const fn with_shuangpin(mut self, enabled: bool) -> Self {
        self.enable_shuangpin = enabled;
        self
    }

    /// 开关全拼首字母匹配。
    #[must_use]
    pub const fn with_full_initials(mut self, enabled: bool) -> Self {
        self.enable_full_initials = enabled;
        self
    }

    /// 开关双拼首字母匹配。
    #[must_use]
    pub const fn with_shuangpin_initials(mut self, enabled: bool) -> Self {
        self.enable_shuangpin_initials = enabled;
        self
    }

    const fn pack(self) -> u32 {
        let mut bits = (self.shuangpin_scheme as u32) & 0xFF;
        if self.enable_full_pinyin {
            bits |= 1 << 8;
        }
        if self.enable_shuangpin {
            bits |= 1 << 9;
        }
        if self.enable_full_initials {
            bits |= 1 << 10;
        }
        if self.enable_shuangpin_initials {
            bits |= 1 << 11;
        }
        bits
    }

    const fn unpack(bits: u32) -> Self {
        Self {
            shuangpin_scheme: ShuangpinScheme::from_index((bits & 0xFF) as usize),
            enable_full_pinyin: (bits & (1 << 8)) != 0,
            enable_shuangpin: (bits & (1 << 9)) != 0,
            enable_full_initials: (bits & (1 << 10)) != 0,
            enable_shuangpin_initials: (bits & (1 << 11)) != 0,
        }
    }
}

/// 全局原子拼音配置槽位（初始值为 `PinyinConfig::new().pack()`，即默认小鹤双拼 + 全模式开启）。
static GLOBAL_PINYIN_CONFIG: AtomicU32 = AtomicU32::new(PinyinConfig::new().pack());

/// 获取当前全局运行时拼音与双拼配置。
#[must_use]
pub fn global_pinyin_config() -> PinyinConfig {
    PinyinConfig::unpack(GLOBAL_PINYIN_CONFIG.load(Ordering::Relaxed))
}

/// 在运行时动态更新全局拼音与双拼配置（对所有使用默认 [`PinyinMatcher::new()`] 的检索点立即生效）。
pub fn set_global_pinyin_config(config: PinyinConfig) {
    GLOBAL_PINYIN_CONFIG.store(config.pack(), Ordering::Relaxed);
}

/// 获取当前全局激活的双拼方案（默认 [`ShuangpinScheme::Xiaohe`]）。
#[must_use]
pub fn global_shuangpin_scheme() -> ShuangpinScheme {
    global_pinyin_config().shuangpin_scheme
}

/// 在运行时动态切换全局双拼方案（如从默认小鹤双拼切换为自然码/微软/搜狗/紫光/智能ABC）。
pub fn set_global_shuangpin_scheme(scheme: ShuangpinScheme) {
    let mut cfg = global_pinyin_config();
    cfg.shuangpin_scheme = scheme;
    set_global_pinyin_config(cfg);
}

/// 单次拼音/双拼/子串匹配结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinyinMatch {
    /// 综合匹配得分（越高越优先：精确前缀 > 子串 > 全拼/双拼 > 首字母缩写 > 离散子序列）。
    pub score: u32,
    /// 命中的原字符串字符索引半开区间 `[start_char, end_char)` 列表（已合并连续区间）。
    pub char_ranges: Vec<(usize, usize)>,
}

/// 汉字全字库全拼、多方案双拼与首字母匹配器。
///
/// * 通过 [`PinyinMatcher::new()`] 创建的实例默认实时读取全局运行时配置（[`global_pinyin_config`]），
///   默认开启小鹤双拼（[`ShuangpinScheme::Xiaohe`]）；
/// * 也可通过 [`PinyinMatcher::with_config`] 或 [`PinyinMatcher::with_scheme`] 绑定实例专属配置，
///   或在运行时调用 [`PinyinMatcher::set_scheme`] / [`PinyinMatcher::set_config`] 动态修改。
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PinyinMatcher {
    config: Option<PinyinConfig>,
}

impl PinyinMatcher {
    /// 创建跟随全局运行时配置的匹配器（默认开启小鹤双拼 + 全拼 + 全拼/双拼首字母）。
    #[must_use]
    pub const fn new() -> Self {
        Self { config: None }
    }

    /// 使用指定的独立配置创建匹配器。
    #[must_use]
    pub const fn with_config(config: PinyinConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    /// 使用指定的双拼方案创建匹配器（其余开关保持默认开启）。
    #[must_use]
    pub const fn with_scheme(scheme: ShuangpinScheme) -> Self {
        Self {
            config: Some(PinyinConfig::new().with_scheme(scheme)),
        }
    }

    /// 当前生效的配置（若未绑定实例级覆盖，则返回 [`global_pinyin_config`]）。
    #[must_use]
    pub fn config(&self) -> PinyinConfig {
        self.config.unwrap_or_else(global_pinyin_config)
    }

    /// 当前生效的双拼方案。
    #[must_use]
    pub fn scheme(&self) -> ShuangpinScheme {
        self.config().shuangpin_scheme
    }

    /// 运行时更新当前匹配器实例的配置。
    pub fn set_config(&mut self, config: PinyinConfig) {
        self.config = Some(config);
    }

    /// 运行时切换当前匹配器实例的双拼方案。
    pub fn set_scheme(&mut self, scheme: ShuangpinScheme) {
        let mut cfg = self.config();
        cfg.shuangpin_scheme = scheme;
        self.config = Some(cfg);
    }

    /// 查询单个汉字的所有合法拼音读音（小写 ASCII，无声调，零堆分配）。
    ///
    /// * 优先检索 `CJK_POLYPHONES` 多音字表；
    /// * 其次 `O(1)` 检索 `U+4E00..=U+9FFF`（20,992 个基本汉字）与 `U+3400..=U+4DBF`（6,592 个扩展 A 区汉字）；
    /// * 支持 `〇`（`U+3007` -> `"ling"`）。
    #[must_use]
    pub fn readings(ch: char) -> &'static [&'static str] {
        if ch == '〇' {
            return &["ling"];
        }
        // 1. 多音字表二分查找
        if let Ok(idx) = CJK_POLYPHONES.binary_search_by_key(&ch, |&(c, _)| c) {
            return CJK_POLYPHONES[idx].1;
        }
        let cp = ch as u32;
        // 2. CJK 基本汉字区 U+4E00..=U+9FFF O(1) 查表
        if (0x4E00..=0x9FFF).contains(&cp) {
            let syl_idx = CJK_UNIFIED_SYLLABLES[(cp - 0x4E00) as usize];
            if syl_idx != INVALID_SYLLABLE {
                return SINGLE_SYLLABLE_READINGS[syl_idx as usize];
            }
        }
        // 3. CJK 扩展 A 区 U+3400..=U+4DBF O(1) 查表
        if (0x3400..=0x4DBF).contains(&cp) {
            let syl_idx = CJK_EXT_A_SYLLABLES[(cp - 0x3400) as usize];
            if syl_idx != INVALID_SYLLABLE {
                return SINGLE_SYLLABLE_READINGS[syl_idx as usize];
            }
        }
        &[]
    }

    /// 查询单个汉字的主拼音读音（若为非汉字则返回 `None`）。
    #[must_use]
    pub fn primary_reading(ch: char) -> Option<&'static str> {
        let cp = ch as u32;
        if (0x4E00..=0x9FFF).contains(&cp) {
            let syl_idx = CJK_UNIFIED_SYLLABLES[(cp - 0x4E00) as usize];
            if syl_idx != INVALID_SYLLABLE {
                return Some(SYLLABLE_STRS[syl_idx as usize]);
            }
        }
        Self::readings(ch).first().copied()
    }

    /// 查询单个汉字在指定双拼方案下的所有 2 字节双拼编码（含多音字读音）。
    #[must_use]
    pub fn shuangpin_codes(ch: char, scheme: ShuangpinScheme) -> Vec<[u8; 2]> {
        let mut out = Vec::new();
        for &py in Self::readings(ch) {
            if let Some(code) = scheme.encode_syllable(py) {
                if !out.contains(&code) {
                    out.push(code);
                }
            }
        }
        out
    }

    /// 对目标文本 `text` 与查询串 `query` 执行多模式匹配（直接子串 + 全拼 + 双拼 + 全拼首字母 + 双拼首字母）。
    #[must_use]
    pub fn match_text(&self, text: &str, query: &str) -> Option<PinyinMatch> {
        let q_trimmed = query.trim();
        if q_trimmed.is_empty() {
            return Some(PinyinMatch {
                score: 100,
                char_ranges: Vec::new(),
            });
        }
        let cfg = self.config();
        let q_lower: String = q_trimmed.chars().flat_map(char::to_lowercase).collect();
        let text_chars: Vec<char> = text.chars().collect();
        let text_lower: Vec<char> = text
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        let q_chars: Vec<char> = q_lower.chars().collect();

        // 1. 直接子串匹配（最高优先级）
        if q_chars.len() <= text_lower.len() {
            for start in 0..=(text_lower.len() - q_chars.len()) {
                if text_lower[start..start + q_chars.len()] == q_chars[..] {
                    let bonus = if start == 0 {
                        1000
                    } else {
                        800 - (start as u32).min(200)
                    };
                    return Some(PinyinMatch {
                        score: bonus,
                        char_ranges: vec![(start, start + q_chars.len())],
                    });
                }
            }
        }

        // 2. 全拼 / 双拼 / 全拼首字母 / 双拼首字母 / 混合连续前缀递归匹配
        for start in 0..text_chars.len() {
            if let Some(end) = Self::match_from(&text_chars, start, &q_lower, cfg) {
                let is_prefix = start == 0;
                let span_len = (end - start).max(1);
                let base_score = if q_lower.len() > span_len {
                    // 全拼或双拼匹配得分高于纯首字母缩写
                    if is_prefix {
                        700
                    } else {
                        550
                    }
                } else if is_prefix {
                    600
                } else {
                    450
                };
                return Some(PinyinMatch {
                    score: base_score - (start as u32).min(100),
                    char_ranges: vec![(start, end)],
                });
            }
        }

        // 3. 离散子序列首字母模糊匹配（如 "czq" 或小鹤双拼首字母 "cvq" 跨分隔符匹配 "材_质_球"）
        //    该分支只服务于「汉字读音首字母缩写」：要求至少一次命中来自汉字读音，且英文
        //    直接命中只允许发生在词首。否则 "accordion" 之类的纯英文单词会被逐字母拆成
        //    "缩写"，误命中任意一段长英文文本。
        if cfg.enable_full_initials || cfg.enable_shuangpin_initials {
            let mut matched_indices = Vec::new();
            let mut q_idx = 0;
            let mut pinyin_initial_hits = 0usize;
            for (c_idx, &ch) in text_chars.iter().enumerate() {
                if q_idx >= q_chars.len() {
                    break;
                }
                let target_q = q_chars[q_idx];
                let target_byte = target_q.is_ascii().then_some(target_q as u8);
                let readings = Self::readings(ch);
                let full_initial_hit = cfg.enable_full_initials
                    && readings
                        .iter()
                        .any(|py| py.as_bytes().first().copied() == target_byte);
                let shuangpin_initial_hit = cfg.enable_shuangpin_initials
                    && readings
                        .iter()
                        .any(|py| cfg.shuangpin_scheme.initial_key(py) == Some(target_q));
                let pinyin_hit = full_initial_hit || shuangpin_initial_hit;
                let at_word_start = c_idx == 0 || !text_chars[c_idx - 1].is_ascii_alphanumeric();
                let direct_hit = at_word_start && ch.to_lowercase().next() == Some(target_q);
                if pinyin_hit || direct_hit {
                    matched_indices.push(c_idx);
                    q_idx += 1;
                    if pinyin_hit {
                        pinyin_initial_hits += 1;
                    }
                }
            }
            if q_idx == q_chars.len() && pinyin_initial_hits > 0 {
                return Some(PinyinMatch {
                    score: 300,
                    char_ranges: coalesce_indices(&matched_indices),
                });
            }
        }

        None
    }

    fn match_from(
        chars: &[char],
        char_idx: usize,
        remaining_query: &str,
        cfg: PinyinConfig,
    ) -> Option<usize> {
        if remaining_query.is_empty() {
            return Some(char_idx);
        }
        if char_idx >= chars.len() {
            return None;
        }
        let ch = chars[char_idx];

        // A. ASCII / 原字符直接匹配
        let mut buf = [0u8; 4];
        let lower_ch = ch.to_lowercase().next().unwrap_or(ch);
        let ch_str: &str = lower_ch.encode_utf8(&mut buf);
        if let Some(rest) = remaining_query.strip_prefix(ch_str) {
            if let Some(end) = Self::match_from(chars, char_idx + 1, rest, cfg) {
                return Some(end);
            }
        }

        let readings = Self::readings(ch);
        if readings.is_empty() {
            return None;
        }

        // B. 汉字全拼或全拼前缀匹配
        if cfg.enable_full_pinyin {
            for &py in readings {
                if let Some(rest) = remaining_query.strip_prefix(py) {
                    if let Some(end) = Self::match_from(chars, char_idx + 1, rest, cfg) {
                        return Some(end);
                    }
                }
                // 末尾音节输入到一半的前缀匹配（长度 >= 2，或单字母作为末尾）
                if py.starts_with(remaining_query) {
                    return Some(char_idx + 1);
                }
            }
        }

        // C. 汉字双拼（2 键）或双拼末尾单键前缀匹配
        if cfg.enable_shuangpin {
            for &py in readings {
                if let Some(pair) = cfg.shuangpin_scheme.encode_syllable(py) {
                    let sp_str = core::str::from_utf8(&pair).unwrap_or("");
                    if !sp_str.is_empty() {
                        if let Some(rest) = remaining_query.strip_prefix(sp_str) {
                            if let Some(end) = Self::match_from(chars, char_idx + 1, rest, cfg) {
                                return Some(end);
                            }
                        }
                        if sp_str.starts_with(remaining_query) {
                            return Some(char_idx + 1);
                        }
                    }
                    // 微软/搜狗/紫光双拼中 `ing` 韵母除 `y` 外支持分号键 `;`
                    if matches!(
                        cfg.shuangpin_scheme,
                        ShuangpinScheme::Microsoft
                            | ShuangpinScheme::Sogou
                            | ShuangpinScheme::Ziguang
                    ) && py.ends_with("ing")
                    {
                        let alt_pair = [pair[0], b';'];
                        if let Ok(alt_str) = core::str::from_utf8(&alt_pair) {
                            if let Some(rest) = remaining_query.strip_prefix(alt_str) {
                                if let Some(end) = Self::match_from(chars, char_idx + 1, rest, cfg)
                                {
                                    return Some(end);
                                }
                            }
                        }
                    }
                }
            }
        }

        // D. 全拼首字母匹配
        if cfg.enable_full_initials {
            for &py in readings {
                if let Some(&first_byte) = py.as_bytes().first() {
                    let first_char = first_byte as char;
                    if let Some(rest) = remaining_query.strip_prefix(first_char) {
                        if let Some(end) = Self::match_from(chars, char_idx + 1, rest, cfg) {
                            return Some(end);
                        }
                    }
                }
            }
        }

        // E. 双拼首字母匹配（例如小鹤双拼 `zh=v, ch=i, sh=u`）
        if cfg.enable_shuangpin_initials {
            for &py in readings {
                if let Some(ini_char) = cfg.shuangpin_scheme.initial_key(py) {
                    if let Some(rest) = remaining_query.strip_prefix(ini_char) {
                        if let Some(end) = Self::match_from(chars, char_idx + 1, rest, cfg) {
                            return Some(end);
                        }
                    }
                }
            }
        }

        None
    }
}

fn coalesce_indices(indices: &[usize]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &idx in indices {
        if let Some(last) = ranges.last_mut() {
            if last.1 == idx {
                last.1 = idx + 1;
                continue;
            }
        }
        ranges.push((idx, idx + 1));
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_all_20992_unified_cjk_ideographs_and_rare_chars() {
        for cp in 0x4E00u32..=0x9FFF {
            let ch = char::from_u32(cp).unwrap();
            let readings = PinyinMatcher::readings(ch);
            assert!(
                !readings.is_empty(),
                "character U+{cp:04X} ({ch}) must have at least one pinyin reading"
            );
        }
        // 生僻字检验：魑魅魍魉、饕餮、鑫森淼焱垚
        let matcher = PinyinMatcher::with_config(PinyinConfig::new());
        assert!(matcher.match_text("魑魅魍魉", "chimeiwangliang").is_some());
        assert!(matcher.match_text("魑魅魍魉", "cmwl").is_some());
        assert!(matcher.match_text("饕餮盛宴", "taotieshengyan").is_some());
        assert!(matcher
            .match_text("鑫森淼焱垚", "xinsenmiaoyanyao")
            .is_some());
    }

    #[test]
    fn full_pinyin_and_initialism_match_chinese_terms() {
        let matcher = PinyinMatcher::with_config(PinyinConfig::new());

        // 全拼匹配
        let m1 = matcher
            .match_text("着色器管线", "zhuoseqi")
            .expect("full pinyin");
        assert_eq!(m1.char_ranges, vec![(0, 3)]);

        // 全拼首字母缩写匹配
        let m2 = matcher
            .match_text("着色器管线", "zsqgx")
            .expect("initialism");
        assert_eq!(m2.char_ranges, vec![(0, 5)]);

        // 多音字匹配："重命名" -> "chongmingming" / "cmm" 和 "重要" -> "zhong"
        assert!(matcher.match_text("重命名文件", "chongming").is_some());
        assert!(matcher.match_text("重命名文件", "cmmwj").is_some());
        assert!(matcher.match_text("重要材质球", "zhongyaoczq").is_some());
    }

    #[test]
    fn xiaohe_shuangpin_is_default_and_matches_codes_and_initials() {
        // 默认开启小鹤双拼
        let matcher = PinyinMatcher::with_config(PinyinConfig::new());
        assert_eq!(matcher.scheme(), ShuangpinScheme::Xiaohe);

        // 小鹤双拼编码验证：
        // "着色器" -> 着(zhuo=vo / zhe=ve) + 色(se=se) + 器(qi=qi)
        assert!(matcher.match_text("着色器管线", "voseqi").is_some());
        assert!(matcher.match_text("着色器管线", "veseqi").is_some());
        // 小鹤双拼首字母：着(zh->v) + 色(s) + 器(q) + 管(g) + 线(x) -> "vsqgx"
        let sp_init = matcher
            .match_text("着色器管线", "vsqgx")
            .expect("xiaohe shuangpin initialism");
        assert_eq!(sp_init.char_ranges, vec![(0, 5)]);

        // "材质球" -> 材(cai=cd) + 质(zhi=vi) + 球(qiu=qq) -> "cdviqq"
        let czq = matcher
            .match_text("材质球_PBR", "cdviqq")
            .expect("xiaohe cdviqq");
        assert_eq!(czq.char_ranges, vec![(0, 3)]);

        // "重命名" -> 重(chong=is) + 命(ming=mk) + 名(ming=mk) -> "ismkmk" 或双拼首字母 "imm"
        assert!(matcher.match_text("重命名文件", "ismkmk").is_some());
        assert!(matcher.match_text("重命名文件", "immwj").is_some());
    }

    #[test]
    fn runtime_switching_across_all_six_shuangpin_schemes_works() {
        let mut matcher = PinyinMatcher::with_config(PinyinConfig::new());

        // 1. 自然码双拼：材(cai=cl) + 质(zhi=vi) + 球(qiu=qq) -> "clviqq"
        matcher.set_scheme(ShuangpinScheme::Natural);
        assert!(matcher.match_text("材质球", "clviqq").is_some());

        // 2. 微软双拼：略(lve=lv)，安(an=oj)，全(quan=qr) -> "ojqr"
        matcher.set_scheme(ShuangpinScheme::Microsoft);
        assert!(matcher.match_text("安全策略", "ojqrcelv").is_some());

        // 3. 搜狗双拼：安(an=oj)，定(ding=dy 或 d;)
        matcher.set_scheme(ShuangpinScheme::Sogou);
        assert!(matcher.match_text("安定", "ojdy").is_some());
        assert!(matcher.match_text("安定", "ojd;").is_some());

        // 4. 紫光双拼：zh=u, ch=a, sh=i -> "着色器" 双拼首字母为 "uisq" 首码 u+s+q -> "usq"
        matcher.set_scheme(ShuangpinScheme::Ziguang);
        assert!(matcher.match_text("着色器", "uoseqi").is_some());
        assert!(matcher.match_text("着色器", "usq").is_some());

        // 5. 智能 ABC 双拼：zh=a, ch=e, sh=v -> "着色器" 为 "aoseqi"，双拼首字母为 "asq"
        matcher.set_scheme(ShuangpinScheme::SmartAbc);
        assert!(matcher.match_text("着色器", "aoseqi").is_some());
        assert!(matcher.match_text("着色器", "asq").is_some());

        // 6. 全局运行时切换测试
        set_global_shuangpin_scheme(ShuangpinScheme::Ziguang);
        let global_matcher = PinyinMatcher::new();
        assert_eq!(global_matcher.scheme(), ShuangpinScheme::Ziguang);
        assert!(global_matcher.match_text("着色器", "uoseqi").is_some());
        // 恢复默认小鹤双拼
        set_global_shuangpin_scheme(ShuangpinScheme::Xiaohe);
        assert_eq!(global_shuangpin_scheme(), ShuangpinScheme::Xiaohe);
    }

    #[test]
    fn direct_substring_scores_higher_than_pinyin() {
        let matcher = PinyinMatcher::new();
        let direct = matcher.match_text("材质球_PBR", "材质").unwrap();
        let pinyin = matcher.match_text("材质球_PBR", "caizhi").unwrap();
        assert!(direct.score > pinyin.score);
        assert_eq!(direct.char_ranges, vec![(0, 2)]);
    }
}
