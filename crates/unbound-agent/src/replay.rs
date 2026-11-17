//! 回放与最小复现。
//!
//! 录下来的动作序列若不能执行，它只是一份日志；能执行，它才是一份
//! **可复现**的程序。最小复现剪枝则把这串程序削到「刚刚好会坏」的长度——
//! 二十步的序列缩到五步，贴进 issue 时对方一眼就能看懂。

use crate::log::{ActionLog, RecordedStep};

/// 最小失败复现。
#[derive(Clone, Debug, PartialEq)]
pub struct MinimalReproduction {
    /// 最小失败子序列。
    pub steps: Vec<RecordedStep>,
    /// 可读的复现脚本。
    pub script: String,
}

impl MinimalReproduction {
    /// 步骤数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// 从动作序列里剪出最小失败子序列。
///
/// 判定交给调用方：`still_fails` 拿到一个候选子序列，回答「它还能复现
/// 问题吗」。剪枝本身只删步骤——删掉之后仍失败的才留下，绝不引入新步骤。
///
/// 算法是 delta debugging（ddmin）：把序列切成 `n` 份，依次尝试
/// 「某一份自己就能复现」「某一份的补集能复现」「相邻两份的并集能复现」，
/// 都不成立就加大粒度重来。相比逐项贪心删除，它能真正收敛到 1-最小解：
/// 贪心会因为删除顺序不同而漏掉更短的答案。
#[must_use]
pub fn minimize_failing_steps<F>(steps: &[RecordedStep], mut still_fails: F) -> Vec<RecordedStep>
where
    F: FnMut(&[RecordedStep]) -> bool,
{
    let mut test: Vec<RecordedStep> = steps.to_vec();
    if !still_fails(&test) {
        // 原序列都复现不了，就没什么可剪的。
        return Vec::new();
    }

    let mut granularity = 2usize;
    while test.len() >= 2 {
        let chunk_size = test.len().div_ceil(granularity);
        let chunks: Vec<Vec<RecordedStep>> = test
            .chunks(chunk_size)
            .map(<[RecordedStep]>::to_vec)
            .collect();

        // 1) 某一份自己就能复现。
        let mut reduced = false;
        for chunk in &chunks {
            if still_fails(chunk) {
                test = chunk.clone();
                granularity = 2;
                reduced = true;
                break;
            }
        }
        if reduced {
            continue;
        }

        // 2) 某一份的补集能复现。
        for index in 0..chunks.len() {
            let complement: Vec<RecordedStep> = chunks
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .flat_map(|(_, chunk)| chunk.iter().cloned())
                .collect();
            if !complement.is_empty() && still_fails(&complement) {
                test = complement;
                granularity = granularity.saturating_sub(1).max(2);
                reduced = true;
                break;
            }
        }
        if reduced {
            continue;
        }

        // 3) 相邻两份的并集能复现。
        if granularity < test.len() && chunks.len() >= 2 {
            for index in 0..chunks.len() - 1 {
                let union: Vec<RecordedStep> = chunks[index]
                    .iter()
                    .chain(chunks[index + 1].iter())
                    .cloned()
                    .collect();
                // 并集不得与原序列等长：等长意味着白跑一圈。
                if union.len() < test.len() && still_fails(&union) {
                    test = union;
                    granularity = granularity.saturating_sub(1).max(2);
                    reduced = true;
                    break;
                }
            }
            if reduced {
                continue;
            }
        }

        // 4) 加大粒度：拆得更细，找不到更短的解就说明当前已经够短。
        if granularity >= test.len() {
            break;
        }
        granularity = (granularity * 2).min(test.len());
    }

    test
}

/// 为一段步骤生成可读脚本。
#[must_use]
pub fn script_of(steps: &[RecordedStep]) -> String {
    ActionLog::script_of(steps)
}
