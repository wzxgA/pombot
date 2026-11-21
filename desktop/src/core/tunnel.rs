// @author wzxgqtl 2026-11-22
// 隧道子进程管理。
//
// 由主进程拉起 cloudflared，好处是用户只需要启动一个程序，退出时隧道也跟着结束。
// 但"正常退出时 kill"不够：主进程被任务管理器强杀或崩溃时子进程会变成孤儿，
// 一直连着公网。因此把子进程放进 Job Object 并设置 KILL_ON_JOB_CLOSE，
// 由系统保证父进程消失时连带回收。

use std::{
    fs::{self, OpenOptions},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use anyhow::{Context, Result, bail};
use windows::Win32::{
    Foundation::HANDLE,
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
};

use crate::config::CloudflareConfig;

/// 启动子进程时不弹控制台窗口，否则用户会看到一个黑框。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 托住隧道子进程。`_job` 必须活到进程结束——
/// 它是最后一道保险：句柄随进程关闭而关闭，系统随即杀掉子进程。
pub struct TunnelGuard {
    child: Child,
    _job: HANDLE,
    pub log_path: PathBuf,
}

impl TunnelGuard {
    // @author wzxgqtl 2026-11-22
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// 主动结束。用于正常退出路径，比等待 Job Object 回收更快。
    // @author wzxgqtl 2026-11-22
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    // @author wzxgqtl 2026-11-22
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// 启动隧道。日志写入 `<数据目录>/logs/cloudflared.log`，
/// 这样排查时一个目录就能看到两端的信息。
// @author wzxgqtl 2026-11-22
pub fn start(config: &CloudflareConfig, data_dir: &Path) -> Result<TunnelGuard> {
    let tunnel = config.tunnel.trim();
    if tunnel.is_empty() {
        bail!("未配置 cloudflare.tunnel（填 `cloudflared tunnel create` 时用的名字）");
    }
    let binary = find_binary(&config.binary)
        .context("找不到 cloudflared，可在 cloudflare.binary 里填绝对路径")?;

    let log_dir = data_dir.join("logs");
    fs::create_dir_all(&log_dir).context("创建日志目录失败")?;
    let log_path = log_dir.join("cloudflared.log");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("打开日志失败：{}", log_path.display()))?;
    let log_for_err = log.try_clone().context("复制日志句柄失败")?;

    let child = Command::new(&binary)
        .args(["tunnel", "run", tunnel])
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_for_err))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("启动 {} 失败", binary.display()))?;

    let job = unsafe { CreateJobObjectW(None, None) }.context("创建 Job Object 失败")?;
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .context("配置 Job Object 失败")?;

    // 必须在子进程开始干活之前挂上去，否则它可能已经建立了外部连接。
    {
        use std::os::windows::io::AsRawHandle;
        let handle = HANDLE(child.as_raw_handle());
        unsafe { AssignProcessToJobObject(job, handle) }.context("把子进程加入 Job Object 失败")?;
    }

    Ok(TunnelGuard {
        child,
        _job: job,
        log_path,
    })
}

/// 按「配置 → 默认安装位置 → PATH」的顺序找可执行文件。
///
/// 不能只依赖 PATH：安装后当前终端往往还拿着旧的环境变量，
/// 表现为"命令行里能跑，程序里起不来"。
// @author wzxgqtl 2026-11-22
pub fn find_binary(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        return path.is_file().then_some(path);
    }

    for candidate in [
        r"C:\Program Files (x86)\cloudflared\cloudflared.exe",
        r"C:\Program Files\cloudflared\cloudflared.exe",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("cloudflared.exe"))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tunnel_name_is_reported_before_spawning() {
        let config = CloudflareConfig {
            auto_start: true,
            tunnel: String::new(),
            binary: String::new(),
        };
        // 用 err() 而不是 expect_err：Ok 类型持有句柄，不便实现 Debug。
        let error = start(&config, Path::new("."))
            .err()
            .expect("空隧道名应报错");
        assert!(error.to_string().contains("cloudflare.tunnel"));
    }

    #[test]
    fn configured_binary_must_exist() {
        // 显式配了路径但文件不存在时，应判定为找不到，
        // 而不是悄悄退回 PATH 去起一个版本不同的程序。
        assert!(find_binary(r"C:\definitely\not\here\cloudflared.exe").is_none());
    }
}
