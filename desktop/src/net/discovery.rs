use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceInfo};

pub fn advertise(port: u16, ip: &str) -> Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new().context("启动 mDNS 服务失败")?;
    let service = ServiceInfo::new(
        "_pombot._tcp.local.",
        "PomBot",
        "pombot.local.",
        ip,
        port,
        &[("version", "1"), ("path", "/ws")][..],
    )
    .context("创建 mDNS 服务信息失败")?
    .enable_addr_auto();
    daemon.register(service).context("广播 mDNS 服务失败")?;
    Ok(daemon)
}
