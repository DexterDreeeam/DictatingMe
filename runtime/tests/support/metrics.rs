//! 把逐条判定汇总成 TPR / FAR，并按对立词档位分列。

use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, Copy)]
pub struct Counter {
    pub total: u32,
    pub woke: u32,
}

impl Counter {
    pub fn record(&mut self, woke: bool) {
        self.total += 1;
        if woke {
            self.woke += 1;
        }
    }

    /// 唤醒率。对正样本是 TPR，对负样本是 FAR。
    pub fn rate(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.woke as f32 / self.total as f32
        }
    }
}

#[derive(Debug, Default)]
pub struct ModeMetrics {
    /// 本人说唤醒词
    pub positive: Counter,
    /// 别人说唤醒词
    pub impostor: Counter,
    /// 本人说对立词，按 T1/T2/T3/T4 分列
    pub confusable: BTreeMap<String, Counter>,
    /// 自由语音（完全无关的句子）
    pub free_speech: Counter,
    /// 正样本按测试条件分列
    pub by_condition: BTreeMap<String, Counter>,
    /// setup 失败的分组
    pub setup_failures: Vec<(String, String)>,
}

impl ModeMetrics {
    pub fn confusable_total(&self) -> Counter {
        let mut total = Counter::default();
        for counter in self.confusable.values() {
            total.total += counter.total;
            total.woke += counter.woke;
        }
        total
    }

    /// 全部负样本合并后的误唤醒率。
    pub fn false_accept(&self) -> Counter {
        let mut total = self.confusable_total();
        total.total += self.impostor.total + self.free_speech.total;
        total.woke += self.impostor.woke + self.free_speech.woke;
        total
    }
}

pub fn percent(value: f32) -> String {
    format!("{:.1}%", value * 100.0)
}
