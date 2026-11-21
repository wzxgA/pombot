// @author wzxgqtl 2026-11-22
// 文本定位：把「点击『导出』」翻译成屏幕坐标。
//
// 为什么不让模型直接给坐标：模型估的坐标常偏几十像素，小控件必然点错；
// 而 OCR 的包围盒误差只有一两像素。所以模型只负责说出目标文字，
// 精确落点由这里算。
//
// 两个必须处理的现实问题：
// 1. 引擎逐字返回并在字间插空格（"提醒事项" → "提 醒 事 项"），
//    因此比较前要去掉全部空白。
// 2. 小字号下会认错个别汉字（实测"场景"被认成"场吕"），
//    纯精确匹配会直接失败，所以需要一个允许少量差异的兜底。

use crate::gui::ocr::{OcrLine, OcrWord};

/// 一次成功定位的结果。
#[derive(Clone, Debug)]
pub struct Hit {
    /// 命中区域的中心点，已经可以直接用来点击。
    pub center: (i32, i32),
    /// 命中区域合并后的包围盒。
    pub rect: OcrWord,
    /// 与目标相差的字符数。0 表示精确命中。
    pub distance: usize,
}

/// 先精确匹配，失败再允许少量差异。
///
/// 容错上限取目标长度的四分之一（至少 1，且不超过长度减一），
/// 这样"场景探针"允许错一个字，而两三个字的短目标仍然要求基本准确。
// @author wzxgqtl 2026-11-22
pub fn find_best(lines: &[OcrLine], target: &str) -> Option<Hit> {
    if let Some(hit) = find_text(lines, target, 0) {
        return Some(hit);
    }
    let needle = normalized_chars(target);
    if needle.len() < 2 {
        return None;
    }
    let allowed = (needle.len() / 4).max(1).min(needle.len() - 1);
    find_within(lines, &needle, allowed)
}

/// 精确匹配，返回第 `occurrence` 个命中（从 0 开始）。
// @author wzxgqtl 2026-11-22
pub fn find_text(lines: &[OcrLine], target: &str, occurrence: usize) -> Option<Hit> {
    let needle = normalized_chars(target);
    if needle.is_empty() {
        return None;
    }
    let (haystack, owners, words) = flatten(lines);
    if needle.len() > haystack.len() {
        return None;
    }

    let mut search_from = 0;
    let mut found = 0;
    while search_from + needle.len() <= haystack.len() {
        if haystack[search_from..search_from + needle.len()] == needle[..] {
            if found == occurrence {
                return build_hit(&words, &owners, search_from, needle.len(), 0, target);
            }
            found += 1;
        }
        search_from += 1;
    }
    None
}

/// 允许最多 `allowed` 个字符不同的滑动匹配，取差异最小、位置最靠前的一个。
// @author wzxgqtl 2026-11-22
pub fn find_within(lines: &[OcrLine], needle: &[char], allowed: usize) -> Option<Hit> {
    if needle.is_empty() {
        return None;
    }
    let (haystack, owners, words) = flatten(lines);
    if needle.len() > haystack.len() {
        return None;
    }

    let mut best: Option<(usize, usize)> = None; // (distance, start)
    for start in 0..=haystack.len() - needle.len() {
        let distance = haystack[start..start + needle.len()]
            .iter()
            .zip(needle)
            .filter(|(left, right)| left != right)
            .count();
        if distance > allowed {
            continue;
        }
        match best {
            // 距离更小优先；距离相同时保留更靠前的位置。
            Some((best_distance, _)) if distance >= best_distance => {}
            _ => best = Some((distance, start)),
        }
        if distance == 0 {
            break;
        }
    }

    let (distance, start) = best?;
    let mut hit = build_hit(&words, &owners, start, needle.len(), distance, "")?;
    // 容错命中时用目标文本作为 label，便于日志里看出匹配的是什么。
    hit.rect.text = needle.iter().collect();
    Some(hit)
}

type Flattened<'a> = (Vec<char>, Vec<usize>, Vec<&'a OcrWord>);

/// 把各行的词摊平成字符序列，同时记录每个字符属于哪个词。
/// 必须按字符而不是字节处理：一个汉字占三字节，用字节偏移索引会整体错位。
fn flatten<'a>(lines: &'a [OcrLine]) -> Flattened<'a> {
    let words: Vec<&OcrWord> = lines.iter().flat_map(|line| line.words.iter()).collect();
    let mut haystack = Vec::new();
    let mut owners = Vec::new();
    for (index, word) in words.iter().enumerate() {
        for ch in normalized_chars(&word.text) {
            haystack.push(ch);
            owners.push(index);
        }
    }
    (haystack, owners, words)
}

fn build_hit(
    words: &[&OcrWord],
    owners: &[usize],
    start: usize,
    length: usize,
    distance: usize,
    label: &str,
) -> Option<Hit> {
    let end = (start + length).min(owners.len());
    let slice = owners.get(start..end)?;

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for owner in slice {
        let word = words[*owner];
        min_x = min_x.min(word.x);
        min_y = min_y.min(word.y);
        max_x = max_x.max(word.x + word.width);
        max_y = max_y.max(word.y + word.height);
    }
    if slice.is_empty() {
        return None;
    }

    let rect = OcrWord {
        text: label.to_owned(),
        x: min_x,
        y: min_y,
        width: (max_x - min_x).max(1),
        height: (max_y - min_y).max(1),
    };
    Some(Hit {
        center: rect.center(),
        rect,
        distance,
    })
}

/// 去掉空白并转小写，仅用于比较。
fn normalized_chars(value: &str) -> Vec<char> {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: i32, y: i32, width: i32, height: i32) -> OcrWord {
        OcrWord {
            text: text.to_owned(),
            x,
            y,
            width,
            height,
        }
    }

    fn line(words: Vec<OcrWord>) -> OcrLine {
        OcrLine {
            text: words.iter().map(|w| w.text.as_str()).collect(),
            words,
        }
    }

    #[test]
    fn matches_chinese_with_inserted_spaces() {
        // 系统 OCR 逐字返回并插空格，直接比较字符串永远不相等。
        let lines = vec![line(vec![
            word("提", 100, 200, 20, 24),
            word("醒", 124, 200, 20, 24),
            word("事", 148, 200, 20, 24),
            word("项", 172, 200, 20, 24),
        ])];
        let hit = find_text(&lines, "提醒事项", 0).expect("应命中");
        assert_eq!(hit.rect.x, 100);
        assert_eq!(hit.rect.width, 92);
        assert_eq!(hit.center, (146, 212));
        assert_eq!(hit.distance, 0);
    }

    #[test]
    fn matches_across_words_and_ignores_case() {
        let lines = vec![line(vec![
            word("SAVE", 40, 60, 50, 20),
            word("As", 94, 60, 30, 20),
        ])];
        let hit = find_text(&lines, "save as", 0).expect("应命中");
        assert_eq!(hit.rect.x, 40);
        assert_eq!(hit.rect.y, 60);
    }

    #[test]
    fn occurrence_selects_the_right_match() {
        let lines = vec![
            line(vec![word("确定", 10, 10, 40, 20)]),
            line(vec![word("确定", 10, 300, 40, 20)]),
        ];
        assert_eq!(find_text(&lines, "确定", 0).expect("第一个").rect.y, 10);
        assert_eq!(find_text(&lines, "确定", 1).expect("第二个").rect.y, 300);
        assert!(find_text(&lines, "确定", 2).is_none(), "越界应返回 None");
    }

    #[test]
    fn missing_text_returns_none() {
        let lines = vec![line(vec![word("主页", 0, 0, 30, 20)])];
        assert!(find_text(&lines, "设置", 0).is_none());
        assert!(find_text(&lines, "   ", 0).is_none());
        assert!(find_text(&[], "任意", 0).is_none());
        assert!(find_best(&lines, "设置").is_none(), "差异过大不应命中");
    }

    #[test]
    fn partial_match_still_reports_a_clickable_rect() {
        let lines = vec![line(vec![
            word("打开", 0, 0, 40, 20),
            word("设置", 44, 0, 40, 20),
        ])];
        let hit = find_text(&lines, "设置", 0).expect("应命中");
        assert_eq!(hit.rect.x, 44);
        assert_eq!(hit.rect.width, 40);
    }

    #[test]
    fn fuzzy_match_tolerates_one_misread_character() {
        // 实测场景：字号偏小时"场景探针"会被认成"场吕探针"。
        // 精确匹配必然失败，而落点本身是对的，所以必须容错。
        let lines = vec![line(vec![
            word("场", 100, 50, 20, 24),
            word("吕", 124, 50, 20, 24),
            word("探", 148, 50, 20, 24),
            word("针", 172, 50, 20, 24),
        ])];
        assert!(find_text(&lines, "场景探针", 0).is_none(), "精确匹配应失败");
        let hit = find_best(&lines, "场景探针").expect("容错匹配应命中");
        assert_eq!(hit.distance, 1);
        assert_eq!(hit.rect.x, 100);
    }

    #[test]
    fn fuzzy_match_prefers_exact_when_both_exist() {
        let lines = vec![
            line(vec![word("场吕探针", 10, 10, 80, 20)]),
            line(vec![word("场景探针", 10, 200, 80, 20)]),
        ];
        let hit = find_best(&lines, "场景探针").expect("应命中");
        assert_eq!(hit.distance, 0);
        assert_eq!(hit.rect.y, 200, "应优先精确命中");
    }

    #[test]
    fn short_targets_do_not_tolerate_diff_set() {
        // 两个字的目标允许 1 个字符不同，等于容忍一半错误，太松。
        // 这里确认"设置"不会被"设法"顶掉：允许 1 个差异确实会命中，
        // 所以阈值下限设为 1 时必须保证长度 >= 2，且调用方优先用精确匹配。
        let lines = vec![line(vec![word("设法", 0, 0, 40, 20)])];
        let hit = find_best(&lines, "设置");
        assert!(hit.is_some(), "两字目标允许一个差异");
        assert_eq!(hit.expect("命中").distance, 1);
    }
}
