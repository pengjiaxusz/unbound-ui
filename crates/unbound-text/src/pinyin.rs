//! 零分配友好的中英文混合拼音与首字母缩写检索引擎（阶段 1 WP3）。
//!
//! 面向 `dunting`（中文资产库、标签、文件夹检索）与 `chahu-render-debugger`（命令面板、
//! 进程名、捕获历史、设置项搜索）：
//! * 支持汉字全拼匹配（如 `"caizhi"` → `"材质球"`、`"zhuoseqi"` → `"着色器"`）；
//! * 支持拼音首字母缩写匹配（如 `"czq"` → `"材质球"`、`"zsq"` → `"着色器"`）；
//! * 支持多音字（如 `"重"` `chong`/`zhong`、`"调"` `tiao`/`diao`、`"行"` `hang`/`xing`）；
//! * 返回匹配得分与精确的字符高亮区间 `Vec<(usize, usize)>`，可直接驱动 UI 高亮染色。

/// 单次拼音/子串匹配结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinyinMatch {
    /// 综合匹配得分（越高越优先：精确前缀 > 子串 > 全拼 > 首字母缩写）。
    pub score: u32,
    /// 命中的原字符串字符索引半开区间 `[start_char, end_char)` 列表（已合并连续区间）。
    pub char_ranges: Vec<(usize, usize)>,
}

/// 汉字拼音词典与多模式匹配器。
#[derive(Copy, Clone, Debug, Default)]
pub struct PinyinMatcher;

impl PinyinMatcher {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 查询单个汉字的所有常见拼音读音（小写 ASCII，无声调）。
    ///
    /// 覆盖常见高频汉字、多音字以及图形渲染/数字资产管理核心词汇。
    #[must_use]
    pub fn readings(ch: char) -> &'static [&'static str] {
        match ch {
            // 多音字优先覆盖
            '重' => &["chong", "zhong"],
            '调' => &["tiao", "diao"],
            '行' => &["hang", "xing"],
            '长' => &["chang", "zhang"],
            '藏' => &["cang", "zang"],
            '传' => &["chuan", "zhuan"],
            '模' => &["mo", "mu"],
            '壳' => &["ke", "qiao"],
            '角' => &["jiao", "jue"],
            '率' => &["lv", "lu", "shuai"],
            '度' => &["du", "duo"],
            '切' => &["qie"],
            '帧' => &["zhen"],
            '渲' => &["xuan"],
            '染' => &["ran"],
            '捕' => &["bu"],
            '获' => &["huo"],
            '试' => &["shi"],
            '着' => &["zhuo", "zhao", "zhe"],
            '色' => &["se", "shai"],
            '器' => &["qi"],
            '材' => &["cai"],
            '质' => &["zhi"],
            '球' => &["qiu"],
            '要' => &["yao"],
            '网' => &["wang"],
            '格' => &["ge"],
            '贴' => &["tie"],
            '图' => &["tu"],
            '纹' => &["wen"],
            '理' => &["li"],
            '型' => &["xing"],
            '资' => &["zi"],
            '产' => &["chan"],
            '库' => &["ku"],
            '文' => &["wen"],
            '件' => &["jian"],
            '夹' => &["jia"],
            '目' => &["mu"],
            '录' => &["lu"],
            '搜' => &["sou"],
            '索' => &["suo"],
            '过' => &["guo"],
            '滤' => &["lv", "lu"],
            '筛' => &["shai"],
            '选' => &["xuan"],
            '排' => &["pai"],
            '序' => &["xu"],
            '标' => &["biao"],
            '签' => &["qian"],
            '收' => &["shou"],
            '星' => &["xing"],
            '导' => &["dao"],
            '入' => &["ru"],
            '出' => &["chu"],
            '设' => &["she"],
            '置' => &["zhi"],
            '主' => &["zhu"],
            '题' => &["ti"],
            '暗' => &["an"],
            '亮' => &["liang"],
            '高' => &["gao"],
            '对' => &["dui"],
            '比' => &["bi"],
            '窗' => &["chuang"],
            '口' => &["kou"],
            '浮' => &["fu"],
            '层' => &["ceng"],
            '透' => &["tou"],
            '明' => &["ming"],
            '穿' => &["chuan"],
            '进' => &["jin"],
            '程' => &["cheng"],
            '启' => &["qi"],
            '动' => &["dong"],
            '注' => &["zhu"],
            '射' => &["she"],
            '挂' => &["gua"],
            '载' => &["zai"],
            '日' => &["ri"],
            '志' => &["zhi"],
            '控' => &["kong"],
            '制' => &["zhi"],
            '台' => &["tai"],
            '性' => &["xing"],
            '能' => &["neng"],
            '内' => &["nei"],
            '存' => &["cun"],
            '显' => &["xian"],
            '管' => &["guan"],
            '线' => &["xian"],
            '命' => &["ming"],
            '令' => &["ling"],
            '面' => &["mian"],
            '板' => &["ban"],
            '快' => &["kuai"],
            '捷' => &["jie"],
            '键' => &["jian"],
            '新' => &["xin"],
            '建' => &["jian"],
            '删' => &["shan"],
            '除' => &["chu"],
            '复' => &["fu"],
            '粘' => &["zhan", "nian"],
            '剪' => &["jian"],
            '名' => &["ming"],
            '刷' => &["shua"],
            '预' => &["yu"],
            '览' => &["lan"],
            '缩' => &["suo"],
            '略' => &["lve", "lue"],
            '放' => &["fang"],
            '大' => &["da", "dai"],
            '小' => &["xiao"],
            '全' => &["quan"],
            '屏' => &["ping"],
            '北' => &["bei"],
            '京' => &["jing"],
            '上' => &["shang"],
            '海' => &["hai"],
            '深' => &["shen"],
            '圳' => &["zhen"],
            '广' => &["guang"],
            '州' => &["zhou"],
            '中' => &["zhong"],
            '英' => &["ying"],
            '语' => &["yu"],
            '言' => &["yan"],
            '缓' => &["huan"],
            '冲' => &["chong"],
            '区' => &["qu"],
            '通' => &["tong"],
            '道' => &["dao"],
            '光' => &["guang"],
            '照' => &["zhao"],
            '阴' => &["yin"],
            '影' => &["ying"],
            '法' => &["fa"],
            '粗' => &["cu"],
            '糙' => &["cao"],
            '金' => &["jin"],
            '属' => &["shu"],
            '环' => &["huan"],
            '境' => &["jing"],
            '遮' => &["zhe"],
            '蔽' => &["bi"],
            '烘' => &["hong"],
            '焙' => &["bei"],
            '画' => &["hua"],
            '骨' => &["gu"],
            '骼' => &["ge"],
            '粒' => &["li"],
            '子' => &["zi"],
            '音' => &["yin"],
            '频' => &["pin"],
            '视' => &["shi"],
            '脚' => &["jiao"],
            '本' => &["ben"],
            '配' => &["pei"],
            '工' => &["gong"],
            '项' => &["xiang"],
            '任' => &["ren"],
            '务' => &["wu"],
            '队' => &["dui"],
            '列' => &["lie"],
            '成' => &["cheng"],
            '功' => &["gong"],
            '失' => &["shi"],
            '败' => &["bai"],
            '警' => &["jing"],
            '告' => &["gao"],
            '错' => &["cuo"],
            '误' => &["wu"],
            '信' => &["xin"],
            '息' => &["xi"],
            '详' => &["xiang"],
            '情' => &["qing"],
            _ => &[],
        }
    }

    /// 对目标文本 `text` 与查询串 `query` 执行多模式匹配（忽略大小写子串 + 全拼 + 首字母缩写）。
    #[must_use]
    pub fn match_text(&self, text: &str, query: &str) -> Option<PinyinMatch> {
        let q_trimmed = query.trim();
        if q_trimmed.is_empty() {
            return Some(PinyinMatch {
                score: 100,
                char_ranges: Vec::new(),
            });
        }
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

        // 2. 全拼 / 首字母缩写 / 混合前缀递归匹配
        for start in 0..text_chars.len() {
            if let Some(end) = Self::match_from(&text_chars, start, &q_lower) {
                let is_prefix = start == 0;
                let span_len = (end - start).max(1);
                let base_score = if q_lower.len() > span_len {
                    // 全拼匹配得分高于纯首字母缩写
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

        // 3. 离散子序列首字母模糊匹配（如 "czq" 跨分隔符匹配 "材_质_球"）
        let mut matched_indices = Vec::new();
        let mut q_idx = 0;
        for (c_idx, &ch) in text_chars.iter().enumerate() {
            if q_idx >= q_chars.len() {
                break;
            }
            let target_q = q_chars[q_idx];
            let direct_hit = ch.to_lowercase().next() == Some(target_q);
            let pinyin_initial_hit = Self::readings(ch)
                .iter()
                .any(|py| py.as_bytes().first().copied() == Some(target_q as u8));
            if direct_hit || pinyin_initial_hit {
                matched_indices.push(c_idx);
                q_idx += 1;
            }
        }
        if q_idx == q_chars.len() && !matched_indices.is_empty() {
            return Some(PinyinMatch {
                score: 300,
                char_ranges: coalesce_indices(&matched_indices),
            });
        }

        None
    }

    fn match_from(chars: &[char], char_idx: usize, remaining_query: &str) -> Option<usize> {
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
            if let Some(end) = Self::match_from(chars, char_idx + 1, rest) {
                return Some(end);
            }
        }
        // B. 汉字拼音全拼或拼音前缀匹配（支持首字母与多音字）
        for &py in Self::readings(ch) {
            if let Some(rest) = remaining_query.strip_prefix(py) {
                if let Some(end) = Self::match_from(chars, char_idx + 1, rest) {
                    return Some(end);
                }
            }
            // 若剩余查询本身是当前汉字拼音的前缀（例如输入到一半 "zsqi" 的最后一个音节 "qi" 或 "q"）
            if py.starts_with(remaining_query) {
                return Some(char_idx + 1);
            }
            // 首字母匹配
            if let Some(first_byte) = py.as_bytes().first() {
                let first_char = *first_byte as char;
                if let Some(rest) = remaining_query.strip_prefix(first_char) {
                    if let Some(end) = Self::match_from(chars, char_idx + 1, rest) {
                        return Some(end);
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
    fn full_pinyin_and_initialism_match_chinese_terms() {
        let matcher = PinyinMatcher::new();

        // 全拼匹配
        let m1 = matcher
            .match_text("着色器管线", "zhuoseqi")
            .expect("full pinyin");
        assert_eq!(m1.char_ranges, vec![(0, 3)]);

        // 首字母缩写匹配
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
    fn direct_substring_scores_higher_than_pinyin() {
        let matcher = PinyinMatcher::new();
        let direct = matcher.match_text("材质球_PBR", "材质").unwrap();
        let pinyin = matcher.match_text("材质球_PBR", "caizhi").unwrap();
        assert!(direct.score > pinyin.score);
        assert_eq!(direct.char_ranges, vec![(0, 2)]);
    }
}
