// @author wzxgqtl 2026-11-20
// 把自定义清单嵌入可执行文件，覆盖 rustc 默认清单里的 DPI 声明。
//
// 不这么做的话进程只能是 SYSTEM_AWARE：rustc 默认嵌入的清单写了 dpiAware=true，
// 而进程的 DPI 感知一旦由清单定型，运行时再调 SetProcessDpiAwarenessContext 会直接失败。
// SYSTEM_AWARE 在多显示器或运行时缩放变更下会导致坐标整体偏移。

fn main() {
    println!("cargo:rerun-if-changed=assets/pombot.manifest");
    println!("cargo:rerun-if-changed=assets/pombot.rc");
    let _ = embed_resource::compile_for_everything("assets/pombot.rc", embed_resource::NONE);
}
