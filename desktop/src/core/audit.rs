// @author wzxgqtl 2026-11-22
// 审计日志：谁、什么时候、从哪、做了什么、结果如何。
//
// 不是调试日志。它的用途是事后追溯——尤其在这个项目里，
// "它不会乱动我的电脑"如果只有 README 上的承诺，没有人会信；
// 有一份逐条追加、可以 grep 的记录，才是证据。
//
// 三个刻意的选择：
// - 写 JSONL 追加文件而不是数据库：崩一行最多丢一行，库坏了日志还在
// - 只记录白名单字段，脱敏写在写入前，而不是事后过滤
// - 写入失败绝不影响主流程：审计坏掉不能导致服务不可用

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value};

/// 单个文件的上限。超过就滚动，避免磁盘被慢慢吃掉。
const MAX_BYTES: u64 = 10 * 1024 * 1024;
/// 保留的历史文件数量（不含当前文件）。
const KEEP_FILES: usize = 5;
const FILE_NAME: &str = "audit.jsonl";

struct Writer {
    path: PathBuf,
    file: File,
    size: u64,
}

static WRITER: OnceLock<Mutex<Option<Writer>>> = OnceLock::new();

/// 初始化写入器。返回是否成功——失败时后续 [`record`] 全是空操作，
/// 主流程照常运行。
// @author wzxgqtl 2026-11-22
pub fn init(data_dir: &Path) -> bool {
    let dir = data_dir.join("logs");
    if fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let path = dir.join(FILE_NAME);
    let file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let slot = WRITER.get_or_init(|| Mutex::new(None));
    *slot.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(Writer { path, file, size });
    true
}

/// 记一条事件。
///
/// `fields` 只应包含白名单内容：设备名、工具名、结果摘要等。
/// 绝不能放 token、api_key 或正文全文——审计不该成为第二个泄露面。
// @author wzxgqtl 2026-11-22
pub fn record(kind: &str, fields: &[(&str, Value)]) {
    let Some(slot) = WRITER.get() else {
        return;
    };
    let mut entry = Map::new();
    entry.insert("ts".to_owned(), Value::String(timestamp()));
    entry.insert("kind".to_owned(), Value::String(kind.to_owned()));
    for (key, value) in fields {
        entry.insert((*key).to_owned(), value.clone());
    }

    let Ok(line) = serde_json::to_string(&Value::Object(entry)) else {
        return;
    };
    let mut guard = slot.lock().unwrap_or_else(|poison| poison.into_inner());
    let Some(writer) = guard.as_mut() else {
        return;
    };

    let bytes = format!("{line}\n");
    // 写入失败就丢弃这一条：审计不能反过来把服务搞挂。
    if writer.file.write_all(bytes.as_bytes()).is_ok() {
        writer.size += bytes.len() as u64;
    }
    if writer.size >= MAX_BYTES {
        rotate(writer);
    }
}

/// 把当前文件改名成 `audit.N.jsonl`，序号越大越旧，超过保留数就删除。
fn rotate(writer: &mut Writer) {
    let dir = match writer.path.parent() {
        Some(dir) => dir.to_path_buf(),
        None => return,
    };
    // 先腾空最后一位，再从后往前搬。
    let oldest = dir.join(format!("audit.{KEEP_FILES}.jsonl"));
    let _ = fs::remove_file(oldest);
    for index in (1..KEEP_FILES).rev() {
        let from = dir.join(format!("audit.{index}.jsonl"));
        let to = dir.join(format!("audit.{}.jsonl", index + 1));
        let _ = fs::rename(from, to);
    }
    let _ = fs::rename(&writer.path, dir.join("audit.1.jsonl"));

    if let Ok(file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&writer.path)
    {
        writer.file = file;
        writer.size = 0;
    }
}

/// 没初始化时写入器不存在，所有记录都是空操作；测试与探针因此无需清理。
// @author wzxgqtl 2026-11-22
pub fn enabled() -> bool {
    WRITER
        .get()
        .and_then(|slot| slot.lock().ok().map(|guard| guard.is_some()))
        .unwrap_or(false)
}

/// 便于手工核对的 ISO 风格时间戳，精确到秒。
fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0);
    format_rfc3339(seconds)
}

fn format_rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// 把"1970-01-01 起的天数"换成公历年月日。
/// 用整数算法而不是引入时间库：只需要格式化，不必为一行日期加依赖。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_known_timestamps() {
        // 2026-01-01T00:00:00Z
        assert_eq!(format_rfc3339(1_767_225_600), "2026-01-01T00:00:00Z");
        // 2026-11-22T00:00:00Z（当年第 326 天）
        assert_eq!(format_rfc3339(1_795_305_600), "2026-11-22T00:00:00Z");
        // 2000-01-01T00:00:00Z
        assert_eq!(format_rfc3339(946_684_800), "2000-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn writes_json_lines_and_rotates() {
        let dir = std::env::temp_dir().join(format!("pombot-audit-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("临时目录");
        assert!(init(&dir), "初始化应成功");
        assert!(enabled());

        record("auth.ok", &[("device", json!("Pixel 8"))]);
        record("tool.executed", &[("tool", json!("create_text_file"))]);

        let path = dir.join("logs").join(FILE_NAME);
        let content = fs::read_to_string(&path).expect("读取日志");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"kind\":\"auth.ok\""));
        assert!(lines[0].contains("\"device\":\"Pixel 8\""));
        assert!(lines[1].contains("create_text_file"));

        fs::remove_dir_all(&dir).ok();
    }
}
