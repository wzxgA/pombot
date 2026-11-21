use std::{env, fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub llm: LlmConfig,
    pub cloudflare: CloudflareConfig,
}

/// 隧道进程的启动方式。
///
/// 由主进程拉起的意义是把生命周期绑在一起：用户只需要启动一个程序，
/// 退出时隧道也跟着结束，不会留下一个还在连着公网的孤儿进程。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudflareConfig {
    /// 是否随主进程启动隧道。关掉它就得自己手动 `cloudflared tunnel run`。
    pub auto_start: bool,
    /// 隧道名称，对应 `cloudflared tunnel create` 时起的名字。
    pub tunnel: String,
    /// `cloudflared` 可执行文件路径。留空则按默认安装位置探测，再退到 PATH。
    pub binary: String,
}

impl Default for CloudflareConfig {
    fn default() -> Self {
        Self {
            // 只在配置了 public_url 时才真正尝试启动，避免没装隧道的用户每次启动都看到告警。
            auto_start: true,
            tunnel: String::new(),
            binary: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// 公网访问地址，例如 `https://pom.example.com`。
    ///
    /// 经 Cloudflare Tunnel 之类反向代理暴露时填写，配对二维码会把它一并下发，
    /// 手机端据此在离开局域网后改用公网地址。留空则只支持局域网。
    pub public_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_owned(),
            port: 8765,
            public_url: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub default: LlmProfile,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmProfile {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub database_file: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let data_dir = match env::var_os("POMBOT_DATA_DIR") {
            Some(value) => PathBuf::from(value),
            None => match env::var_os("APPDATA") {
                Some(value) => PathBuf::from(value).join("PomBot"),
                None => env::current_dir().context("无法确定当前目录")?.join("data"),
            },
        };
        let config_file = match env::var_os("POMBOT_CONFIG") {
            Some(value) => PathBuf::from(value),
            None => data_dir.join("config.toml"),
        };
        Ok(Self {
            database_file: data_dir.join("pombot.db"),
            data_dir,
            config_file,
        })
    }
}

impl AppConfig {
    pub fn load(paths: &AppPaths) -> Result<Self> {
        if let Some(parent) = paths.data_dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::create_dir_all(&paths.data_dir)
            .with_context(|| format!("无法创建数据目录 {}", paths.data_dir.display()))?;
        let _ = dotenvy::from_filename(paths.data_dir.join(".env"));
        let _ = dotenvy::dotenv();

        let mut config = if paths.config_file.exists() {
            let text = fs::read_to_string(&paths.config_file)
                .with_context(|| format!("无法读取配置 {}", paths.config_file.display()))?;
            toml::from_str::<Self>(&text).with_context(|| {
                format!(
                    "配置格式错误：{}。请参考 README 的 llm.default 示例",
                    paths.config_file.display()
                )
            })?
        } else {
            Self::default()
        };

        if let Ok(value) = env::var("POMBOT_LLM_BASE_URL") {
            config.llm.default.base_url = value;
        }
        if let Ok(value) = env::var("POMBOT_LLM_API_KEY") {
            config.llm.default.api_key = value;
        }
        if let Ok(value) = env::var("POMBOT_LLM_MODEL") {
            config.llm.default.model = value;
        }
        if let Ok(value) = env::var("POMBOT_PORT") {
            config.server.port = value
                .parse()
                .with_context(|| format!("POMBOT_PORT 不是有效端口：{value}"))?;
        }
        Ok(config)
    }

    pub fn llm_problem(&self) -> Option<String> {
        let profile = &self.llm.default;
        if !profile.base_url.trim().is_empty() && !profile.model.trim().is_empty() {
            return None;
        }
        Some(format!(
            "尚未配置模型。请编辑 {} 并填入：\n[llm]\ndefault = {{ base_url = \"https://api.openai.com/v1\", api_key = \"你的密钥\", model = \"gpt-4o-mini\" }}\n也可以在数据目录的 .env 文件中设置 POMBOT_LLM_BASE_URL、POMBOT_LLM_API_KEY、POMBOT_LLM_MODEL，然后重启 PomBot。",
            self_path_display()
        ))
    }
}

fn self_path_display() -> String {
    match env::var_os("POMBOT_CONFIG") {
        Some(value) => PathBuf::from(value).display().to_string(),
        None => AppPaths::discover()
            .map(|paths| paths.config_file.display().to_string())
            .unwrap_or_else(|_| "config.toml".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inline_llm_profile() {
        let config: AppConfig = toml::from_str(
            r#"
            [server]
            port = 9000
            [llm]
            default = { base_url = "http://localhost:11434/v1", api_key = "", model = "qwen3" }
            "#,
        )
        .expect("配置应可解析");
        assert_eq!(config.server.port, 9000);
        assert_eq!(config.llm.default.model, "qwen3");
        assert!(config.llm_problem().is_none());
    }

    #[test]
    fn missing_model_has_actionable_error() {
        let problem = AppConfig::default().llm_problem().expect("应提示缺配置");
        assert!(problem.contains("llm"));
        assert!(problem.contains("POMBOT_LLM_API_KEY"));
    }
}
