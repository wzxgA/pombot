// @author wzxgqtl 2026-11-22
// 步进验证器：回答"刚才那一步到底生效了吗"。
//
// GUI 自动化最致命的失败不是报错，而是**静默无效**：点偏了几个像素，
// 界面毫无变化，但流程继续往下走，最后拿到一个莫名其妙的结果。
// 所以每一步之后都必须能给出一个确定的判断。
//
// 阈值刻意取小：光标闪烁、时钟跳秒都会产生真实但微小的变化，
// 只有同时满足"结构变了"和"变化面积够大"才算一步生效。

use crate::gui::{
    capture::Shot,
    digest::{changed_ratio, dhash, hamming},
};

/// 汉明距离超过它才算结构变化。0~2 通常是抗锯齿与压缩噪声。
const HASH_THRESHOLD: u32 = 2;
/// 变化面积占比下限。低于 0.2% 的多半是光标或单个字符的抖动。
const RATIO_THRESHOLD: f32 = 0.002;
/// 连续多少步没有变化就判定卡死。
const STAGNATION_LIMIT: usize = 3;

/// 一次观察：画面 + 指纹。指纹只算一次，反复比较不必重算。
#[derive(Clone)]
pub struct Observation {
    pub shot: Shot,
    pub hash: u64,
}

impl Observation {
    // @author wzxgqtl 2026-11-22
    pub fn new(shot: Shot) -> Self {
        let hash = dhash(&shot);
        Self { shot, hash }
    }
}

/// 一步的变化判定。
#[derive(Clone, Copy, Debug)]
pub struct Verdict {
    pub changed: bool,
    pub hamming: u32,
    pub ratio: f32,
}

/// 比较前后两次观察。
// @author wzxgqtl 2026-11-22
pub fn compare(before: &Observation, after: &Shot) -> Verdict {
    let hamming = hamming(before.hash, dhash(after));
    let ratio = changed_ratio(&before.shot, after);
    Verdict {
        // 两个条件都要满足：只看面积会把整屏换页但配色相近的情况漏掉，
        // 只看指纹会被大面积纯色区域的小改动骗过。
        changed: hamming > HASH_THRESHOLD && ratio > RATIO_THRESHOLD,
        hamming,
        ratio,
    }
}

/// 连续无变化的计数器，用于中止卡死的流程。
///
/// 没有它，一个点不中的按钮会让流程安静地空转，用户只看到指示灯一直转。
#[derive(Default)]
pub struct Stagnation {
    streak: usize,
}

impl Stagnation {
    /// 记录一次判定，返回是否应当中止。
    // @author wzxgqtl 2026-11-22
    pub fn record(&mut self, changed: bool) -> bool {
        if changed {
            self.streak = 0;
            return false;
        }
        self.streak += 1;
        self.streak >= STAGNATION_LIMIT
    }

    // @author wzxgqtl 2026-11-22
    pub fn streak(&self) -> usize {
        self.streak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, value: u8) -> Shot {
        Shot {
            width,
            height,
            bgra: vec![value; width * height * 4],
        }
    }

    fn patched(width: usize, height: usize, base: u8, rect: (usize, usize, usize, usize)) -> Shot {
        let mut shot = solid(width, height, base);
        for y in rect.1..rect.1 + rect.3 {
            for x in rect.0..rect.0 + rect.2 {
                let offset = (y * width + x) * 4;
                shot.bgra[offset..offset + 4].fill(10);
            }
        }
        shot
    }

    #[test]
    fn identical_frames_are_not_changes() {
        let before = Observation::new(solid(320, 200, 240));
        let verdict = compare(&before, &solid(320, 200, 240));
        assert!(!verdict.changed);
        assert_eq!(verdict.hamming, 0);
    }

    #[test]
    fn cursor_blink_is_not_a_step_of_progress() {
        // 一个 4x4 的亮点，面积占比极小：真实存在，但不该被当成"操作生效"。
        let before = Observation::new(solid(320, 200, 240));
        let after = patched(320, 200, 240, (100, 100, 4, 4));
        let verdict = compare(&before, &after);
        assert!(verdict.ratio > 0.0);
        assert!(!verdict.changed, "微小变化不应判定为生效");
    }

    #[test]
    fn large_layout_change_is_detected() {
        let before = Observation::new(patched(320, 200, 240, (20, 20, 100, 60)));
        let after = patched(320, 200, 240, (180, 120, 100, 60));
        assert!(compare(&before, &after).changed);
    }

    #[test]
    fn stagnation_stops_after_three_quiet_steps() {
        let mut stagnation = Stagnation::default();
        assert!(!stagnation.record(false));
        assert!(!stagnation.record(false));
        assert!(stagnation.record(false), "第三步应触发中止");
        assert_eq!(stagnation.streak(), 3);
    }

    #[test]
    fn progress_resets_the_streak() {
        let mut stagnation = Stagnation::default();
        stagnation.record(false);
        stagnation.record(false);
        assert!(!stagnation.record(true));
        assert_eq!(stagnation.streak(), 0);
        assert!(!stagnation.record(false));
    }
}
