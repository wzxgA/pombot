// @author wzxgqtl 2026-11-22
// 图像指纹与差异度量。
//
// 验证器每步都要回答“界面真的变了吗”，这件事不能用模型来做：
// 慢、贵，而且同一张图两次判断可能不一致。这里提供的都是纯函数，
// 输入相同必得相同结果，因此判据可以写成断言。
//
// 两种度量分工不同：
// - 汉明距离（dHash）：对整体结构变化敏感，抗轻微渲染差异
// - 变化面积占比：对局部内容变化敏感，但会被动画与光标干扰

use crate::gui::capture::Shot;

/// dHash 的网格：宽 9 高 8，相邻两列比较后正好得到 64 位。
const HASH_WIDTH: usize = 9;
const HASH_HEIGHT: usize = 8;

/// 判定“这个像素变了”的亮度差阈值。
/// 取 8 是有意的：压缩噪声与抗锯齿抖动通常在 3 以内，而文字变化远超 20。
const LUMA_DELTA: i32 = 8;

/// 计算 64 位差分哈希。
///
/// 先把图降采样到 9×8 的灰度网格（盒式平均，避免单点噪声影响结果），
/// 再逐行比较相邻像素的大小关系。因此它对整体明暗变化不敏感，
/// 但对布局与内容变化敏感，适合判断“还是不是同一个界面”。
pub fn dhash(shot: &Shot) -> u64 {
    let grid = gray_grid(shot);
    let mut hash = 0_u64;
    for row in 0..HASH_HEIGHT {
        for column in 0..HASH_WIDTH - 1 {
            let left = grid[row * HASH_WIDTH + column];
            let right = grid[row * HASH_WIDTH + column + 1];
            hash <<= 1;
            if left > right {
                hash |= 1;
            }
        }
    }
    hash
}

/// 两个指纹的汉明距离，范围 0..=64。
/// 经验值：0~2 视为同一画面，3~10 属于局部变化，大于 20 基本是换了界面。
pub fn hamming(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

/// 亮度不同的像素占比，范围 0.0~1.0。
///
/// 尺寸不一致时直接返回 1.0：宁可判成“变了”，也不要拿两张不同尺寸的图
/// 逐像素比较得出一个看着合理但毫无意义的数字。
pub fn changed_ratio(left: &Shot, right: &Shot) -> f32 {
    if left.width != right.width || left.height != right.height {
        return 1.0;
    }
    let total = left.width * left.height;
    if total == 0 {
        return 0.0;
    }
    let mut changed = 0_usize;
    for index in 0..total {
        let offset = index * 4;
        if (luma(&left.bgra[offset..offset + 4]) - luma(&right.bgra[offset..offset + 4])).abs()
            > LUMA_DELTA
        {
            changed += 1;
        }
    }
    changed as f32 / total as f32
}

/// 降采样到 9×8 的灰度网格。源图比网格还小时按边缘像素扩展，
/// 保证任何尺寸的截图都能算出指纹。
fn gray_grid(shot: &Shot) -> Vec<i32> {
    let mut grid = vec![0_i32; HASH_WIDTH * HASH_HEIGHT];
    for row in 0..HASH_HEIGHT {
        for column in 0..HASH_WIDTH {
            let x0 = column * shot.width / HASH_WIDTH;
            let x1 = ((column + 1) * shot.width / HASH_WIDTH)
                .max(x0 + 1)
                .min(shot.width);
            let y0 = row * shot.height / HASH_HEIGHT;
            let y1 = ((row + 1) * shot.height / HASH_HEIGHT)
                .max(y0 + 1)
                .min(shot.height);

            let mut sum = 0_i64;
            let mut count = 0_i64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let offset = (y * shot.width + x) * 4;
                    sum += i64::from(luma(&shot.bgra[offset..offset + 4]));
                    count += 1;
                }
            }
            grid[row * HASH_WIDTH + column] = if count == 0 { 0 } else { (sum / count) as i32 };
        }
    }
    grid
}

/// BGRA -> 亮度（Rec.601 近似）。整数运算足够，且不引入浮点误差。
fn luma(pixel: &[u8]) -> i32 {
    let blue = i32::from(pixel[0]);
    let green = i32::from(pixel[1]);
    let red = i32::from(pixel[2]);
    (red * 299 + green * 587 + blue * 114) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一张纯色图，可选在某块区域涂上不同亮度。
    fn solid(width: usize, height: usize, value: u8) -> Shot {
        Shot {
            width,
            height,
            bgra: vec![value; width * height * 4],
        }
    }

    fn with_patch(
        width: usize,
        height: usize,
        base: u8,
        patch: (usize, usize, usize, usize),
        value: u8,
    ) -> Shot {
        let mut shot = solid(width, height, base);
        for y in patch.1..patch.1 + patch.3 {
            for x in patch.0..patch.0 + patch.2 {
                let offset = (y * width + x) * 4;
                shot.bgra[offset..offset + 4].fill(value);
            }
        }
        shot
    }

    #[test]
    fn identical_images_have_zero_distance() {
        let first = with_patch(320, 200, 240, (40, 30, 120, 60), 20);
        let second = first.clone();
        assert_eq!(dhash(&first), dhash(&second));
        assert_eq!(hamming(dhash(&first), dhash(&second)), 0);
        assert_eq!(changed_ratio(&first, &second), 0.0);
    }

    #[test]
    fn layout_change_is_detected() {
        // 同一块暗区从左上移到右下：整体信息量相同，但结构变了。
        let first = with_patch(320, 200, 240, (20, 20, 120, 60), 20);
        let second = with_patch(320, 200, 240, (180, 110, 120, 60), 20);
        assert!(hamming(dhash(&first), dhash(&second)) > 0);
        assert!(changed_ratio(&first, &second) > 0.05);
    }

    #[test]
    fn tiny_patch_change_stays_small_but_nonzero() {
        // 光标闪烁这类微小变化应当被识别为“变了”，但占比很小，
        // 因此调用方可以用阈值把它和“界面切换”区分开。
        let first = solid(320, 200, 240);
        let second = with_patch(320, 200, 240, (100, 100, 8, 8), 0);
        let ratio = changed_ratio(&first, &second);
        assert!(ratio > 0.0, "微小变化也应被察觉");
        assert!(ratio < 0.01, "占比应当很小，实际 {ratio}");
    }

    #[test]
    fn size_mismatch_counts_as_full_change() {
        let first = solid(100, 100, 10);
        let second = solid(101, 100, 10);
        assert_eq!(changed_ratio(&first, &second), 1.0);
    }

    #[test]
    fn hash_works_on_tiny_images() {
        // 比网格还小的图不应 panic，也不应出现除零。
        let shot = solid(3, 2, 128);
        let _ = dhash(&shot);
        assert_eq!(changed_ratio(&shot, &shot), 0.0);
    }
}
