//! Text Diff Engine（见 brainstrom/plan.md §8.1）。
//!
//! 对比"上次完整识别文本"与"当前完整识别文本"，只取新增后缀，不做修正回退——
//! 如果模型后续修正了之前的识别结果，DM 不会退格重打，只会继续追加新内容。

/// 文本增量对比引擎，每次听写会话（一次 Loading→Unloading 周期）持有一个实例。
pub struct TextDiffEngine {
    last_full_text: String,
    emitted_reference_text: String,
}

impl TextDiffEngine {
    pub fn new() -> Self {
        Self {
            last_full_text: String::new(),
            emitted_reference_text: String::new(),
        }
    }

    /// 输入当前完整识别文本，返回需要新增打字的后缀（可能为空字符串）。
    /// 不做"选中删除重打"，只做纯后缀追加。
    pub fn compute_suffix(&mut self, current_full_text: &str) -> String {
        let direct_suffix = current_full_text.strip_prefix(&self.last_full_text);
        let reference_suffix =
            suffix_after_reference(&self.emitted_reference_text, current_full_text);
        let suffix = match reference_suffix {
            Some(suffix) if !suffix.is_empty() => suffix,
            Some(_) => "",
            None if !self
                .emitted_reference_text
                .starts_with(&self.last_full_text) =>
            {
                direct_suffix.unwrap_or_default()
            }
            None => "",
        };

        self.last_full_text.clear();
        self.last_full_text.push_str(current_full_text);
        if !suffix.is_empty() {
            self.emitted_reference_text.clear();
            self.emitted_reference_text.push_str(current_full_text);
        }
        suffix.to_owned()
    }

    /// 一次听写结束后重置内部状态（`State::Unloading` 阶段调用）。
    pub fn reset(&mut self) {
        self.last_full_text.clear();
        self.emitted_reference_text.clear();
    }

    pub(crate) fn final_full_text(&self) -> &str {
        &self.last_full_text
    }
}

fn suffix_after_reference<'a>(reference: &str, current: &'a str) -> Option<&'a str> {
    if let Some(suffix) = current.strip_prefix(reference) {
        return Some(suffix);
    }

    for (start, _) in reference.char_indices() {
        let anchor = &reference[start..];
        let starts_at_token_boundary = start == 0
            || anchor.chars().next().is_some_and(|character| {
                character.is_whitespace() || character.is_ascii_punctuation()
            })
            || reference[..start]
                .chars()
                .next_back()
                .is_some_and(|character| {
                    character.is_whitespace() || character.is_ascii_punctuation()
                });
        if !starts_at_token_boundary && anchor.chars().count() < 2 {
            continue;
        }

        if let Some(anchor_start) = current.rfind(anchor) {
            return Some(&current[anchor_start + anchor.len()..]);
        }
    }
    None
}

impl Default for TextDiffEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_only_utf8_safe_appended_suffixes() {
        let mut diff = TextDiffEngine::new();
        assert_eq!(diff.compute_suffix("你好"), "你好");
        assert_eq!(diff.compute_suffix("你好，世"), "，世");
        assert_eq!(diff.compute_suffix("你好，世界🌍"), "界🌍");
        assert_eq!(diff.final_full_text(), "你好，世界🌍");
    }

    #[test]
    fn revisions_never_backspace_or_duplicate_injected_text() {
        let mut diff = TextDiffEngine::new();
        assert_eq!(diff.compute_suffix("I like cats"), "I like cats");
        assert_eq!(diff.compute_suffix("I love cats and dogs"), " and dogs");
        assert_eq!(diff.compute_suffix("I love cats and dogs today"), " today");
        assert_eq!(diff.final_full_text(), "I love cats and dogs today");
    }

    #[test]
    fn unrelated_revisions_are_not_injected() {
        let mut diff = TextDiffEngine::new();
        diff.compute_suffix("turn left");
        assert_eq!(diff.compute_suffix("proceed tomorrow"), "");
        assert_eq!(
            diff.compute_suffix("proceed tomorrow carefully"),
            " carefully"
        );
        assert_eq!(diff.final_full_text(), "proceed tomorrow carefully");
    }

    #[test]
    fn rollback_then_regrowth_does_not_duplicate_injected_text() {
        let mut diff = TextDiffEngine::new();
        assert_eq!(diff.compute_suffix("hello world"), "hello world");
        assert_eq!(diff.compute_suffix("hello"), "");
        assert_eq!(diff.compute_suffix("hello world again"), " again");
        assert_eq!(diff.compute_suffix(""), "");
        assert_eq!(diff.compute_suffix("hello world again!"), "!");
        assert_eq!(diff.final_full_text(), "hello world again!");
    }

    #[test]
    fn reset_starts_a_fresh_session() {
        let mut diff = TextDiffEngine::new();
        diff.compute_suffix("old");
        diff.reset();
        assert_eq!(diff.compute_suffix("new"), "new");
    }
}
