// @author wzxgqtl 2026-11-21
// 可被模型调用的工具。
//
// 目前只有一个「写文本文件」——它不碰鼠标键盘、不碰视觉，风险最低，
// 但正好把 Agent Loop 的结构跑通。后续 M4 的 GUI 工具直接往 execute 里注册即可。

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{contract::Risk, core::llm::ToolDef, store::Store};

/// 生成的中间文件目录名。产物实体由 blobs 目录持有，这里只是写盘时的落脚点。
const GENERATED_DIR: &str = "generated";
/// 文件名上限。过长会让部分文件系统报错，也会让手机端列表难以阅读。
const MAX_NAME_CHARS: usize = 80;

pub fn specs() -> Vec<ToolDef> {
    vec![ToolDef {
        kind: "function",
        function: crate::core::llm::ToolFunctionDef {
            name: "create_text_file",
            description: "把一段文本保存为文件并登记成产物，用户会在手机端的「产物」列表里看到它。\
                          当用户要求生成 md、txt、json、csv 之类的文本文件时使用。",
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "文件名，需带扩展名，例如「周报.md」"
                    },
                    "content": {
                        "type": "string",
                        "description": "文件的完整内容"
                    }
                },
                "required": ["name", "content"]
            }),
        },
    }]
}

/// 工具的风险等级。
///
/// 判定标准是四条，满足任意一条就往上升级：
/// 可撤销性、外部副作用、是否涉及钱财与身份、数据破坏性。
/// 未注册的工具一律按 Blocked 处理，避免出现"忘了标注就等于放行"。
pub fn risk_of(name: &str) -> Risk {
    match name {
        // 只写本地文件，错了删掉即可，不产生外部影响。
        "create_text_file" => Risk::Low,
        _ => Risk::Blocked,
    }
}

/// 给用户看的一句话说明。确认弹窗必须能让人判断"它到底要做什么"，
/// 只显示工具名是没有意义的。
pub fn summary_of(name: &str, arguments: &str) -> String {
    match name {
        "create_text_file" => {
            let file = serde_json::from_str::<Value>(arguments)
                .ok()
                .and_then(|args| args.get("name").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| "未命名文件".to_owned());
            format!("创建文件「{file}」")
        }
        other => format!("调用工具 {other}"),
    }
}

/// 执行一次工具调用，返回给模型看的结果文本。
pub fn execute(name: &str, arguments: &str, store: &Store, data_dir: &Path) -> Result<String> {
    match name {
        "create_text_file" => create_text_file(arguments, store, data_dir),
        other => bail!("未知工具：{other}"),
    }
}

fn create_text_file(arguments: &str, store: &Store, data_dir: &Path) -> Result<String> {
    let args: Value = serde_json::from_str(arguments).context("工具参数不是合法 JSON")?;
    let name = args.get("name").and_then(Value::as_str).unwrap_or("");
    let content = args.get("content").and_then(Value::as_str).unwrap_or("");
    if content.trim().is_empty() {
        bail!("content 不能为空");
    }

    let file_name = sanitize_name(name);
    let dir = data_dir.join(GENERATED_DIR);
    fs::create_dir_all(&dir).context("无法创建生成目录")?;
    let path = dir.join(&file_name);
    fs::write(&path, content).context("写入文件失败")?;

    let record = crate::core::artifacts::register_artifact(store, data_dir, &path)?;
    Ok(format!(
        "已生成产物「{}」，共 {} 字符。用户可在手机端「产物」中打开查看。",
        record.title,
        content.chars().count()
    ))
}

/// 只保留文件名部分，去掉路径分隔符与 Windows 不允许的字符。
/// 模型偶尔会给出 `../x.md` 或 `C:\x.md`，必须挡住。
fn sanitize_name(raw: &str) -> String {
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('.');
    let cleaned: String = base
        .chars()
        .filter(|ch| !matches!(ch, ':' | '*' | '?' | '"' | '<' | '>' | '|') && !ch.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return "pombot.md".to_owned();
    }
    if Path::new(cleaned).extension().is_some() {
        cleaned.to_owned()
    } else {
        format!("{cleaned}.md")
    }
}

/// 供测试与后续调用：生成文件的实际落盘位置。
pub fn generated_path(data_dir: &Path, file_name: &str) -> PathBuf {
    data_dir.join(GENERATED_DIR).join(sanitize_name(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_dangerous_names() {
        assert_eq!(sanitize_name("周报.md"), "周报.md");
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd.md");
        assert_eq!(sanitize_name("C:/Windows/x.txt"), "x.txt");
        assert_eq!(sanitize_name("a:b*c?.md"), "abc.md");
        assert_eq!(sanitize_name(""), "pombot.md");
        assert_eq!(sanitize_name("无扩展名"), "无扩展名.md");
    }

    #[test]
    fn create_text_file_registers_an_artifact() {
        let store = crate::store::Store::in_memory().expect("内存库");
        let dir = std::env::temp_dir().join(format!("pombot-tools-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("临时目录");

        let arguments = json!({
            "name": "周报.md",
            "content": "# 本周\n- 完成 M2"
        })
        .to_string();

        let result = execute("create_text_file", &arguments, &store, &dir).expect("执行工具");
        assert!(result.contains("周报.md"));

        let objects = store.objects_since(0, 10).expect("读取对象");
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].title, "周报.md");
        assert!(objects[0].artifact.is_some(), "产物元数据应写入对象");

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unregistered_tools_are_blocked_by_default() {
        // 关键行为：没有显式登记的工具一律不放行，避免"忘了标注就等于允许"。
        assert_eq!(risk_of("create_text_file"), Risk::Low);
        assert_eq!(risk_of("run_command"), Risk::Blocked);
        assert_eq!(risk_of(""), Risk::Blocked);
    }

    #[test]
    fn summary_mentions_the_concrete_target() {
        let arguments = json!({ "name": "周报.md", "content": "x" }).to_string();
        let summary = summary_of("create_text_file", &arguments);
        assert!(summary.contains("周报.md"), "实际：{summary}");
    }

    #[test]
    fn rejects_unknown_tool() {
        let store = crate::store::Store::in_memory().expect("内存库");
        let result = execute("rm_rf", "{}", &store, Path::new("."));
        assert!(result.is_err());
    }
}
