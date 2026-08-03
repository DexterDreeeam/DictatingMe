//! 把 E2E 结果写到 `assets/reports/`，同时在 stdout 打一份可读表格。

use std::io::Write;

use super::corpus::assets_dir;
use super::metrics::{percent, ModeMetrics};

pub fn print_and_persist(mode: &str, metrics: &ModeMetrics) {
    println!("\n================ 模式 {mode} ================");
    println!("{:<28} {:>7} {:>7} {:>9}", "分类", "样本", "唤醒", "比率");
    let row = |label: &str, counter: &super::metrics::Counter| {
        println!(
            "{:<28} {:>7} {:>7} {:>9}",
            label,
            counter.total,
            counter.woke,
            percent(counter.rate())
        );
    };
    row("正样本 (TPR)", &metrics.positive);
    for (tier, counter) in &metrics.confusable {
        row(&format!("对立词 {tier} (FAR)"), counter);
    }
    row("对立词合计 (FAR)", &metrics.confusable_total());
    row("冒充者 (FAR)", &metrics.impostor);
    row("自由语音 (FAR)", &metrics.free_speech);
    row("负样本合计 (FAR)", &metrics.false_accept());

    if !metrics.by_condition.is_empty() {
        println!("\n-- 正样本按条件分列 --");
        for (condition, counter) in &metrics.by_condition {
            row(condition, counter);
        }
    }

    if !metrics.setup_failures.is_empty() {
        println!("\n-- setup 失败 --");
        for (group, reason) in &metrics.setup_failures {
            println!("  {group}: {reason}");
        }
    }

    if let Err(error) = persist(mode, metrics) {
        eprintln!("[e2e] failed to write report: {error}");
    }
}

fn persist(mode: &str, metrics: &ModeMetrics) -> std::io::Result<()> {
    let dir = assets_dir().join("reports");
    std::fs::create_dir_all(&dir)?;
    let mut file = std::fs::File::create(dir.join(format!("{mode}.json")))?;
    let mut confusable = String::new();
    for (tier, counter) in &metrics.confusable {
        if !confusable.is_empty() {
            confusable.push(',');
        }
        confusable.push_str(&format!(
            "\"{tier}\":{{\"total\":{},\"woke\":{},\"rate\":{:.4}}}",
            counter.total,
            counter.woke,
            counter.rate()
        ));
    }
    let mut conditions = String::new();
    for (condition, counter) in &metrics.by_condition {
        if !conditions.is_empty() {
            conditions.push(',');
        }
        conditions.push_str(&format!(
            "\"{condition}\":{{\"total\":{},\"woke\":{},\"rate\":{:.4}}}",
            counter.total,
            counter.woke,
            counter.rate()
        ));
    }
    let false_accept = metrics.false_accept();
    write!(
        file,
        concat!(
            "{{\"mode\":\"{}\",",
            "\"tpr\":{{\"total\":{},\"woke\":{},\"rate\":{:.4}}},",
            "\"impostor\":{{\"total\":{},\"woke\":{},\"rate\":{:.4}}},",
            "\"freeSpeech\":{{\"total\":{},\"woke\":{},\"rate\":{:.4}}},",
            "\"falseAccept\":{{\"total\":{},\"woke\":{},\"rate\":{:.4}}},",
            "\"confusableByTier\":{{{}}},",
            "\"positiveByCondition\":{{{}}},",
            "\"setupFailures\":{}}}"
        ),
        mode,
        metrics.positive.total,
        metrics.positive.woke,
        metrics.positive.rate(),
        metrics.impostor.total,
        metrics.impostor.woke,
        metrics.impostor.rate(),
        metrics.free_speech.total,
        metrics.free_speech.woke,
        metrics.free_speech.rate(),
        false_accept.total,
        false_accept.woke,
        false_accept.rate(),
        confusable,
        conditions,
        metrics.setup_failures.len()
    )
}
