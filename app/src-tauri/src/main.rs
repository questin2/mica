// 始终以「窗口子系统」构建：双击启动时**不再弹出黑色命令窗**（Windows 专属开关）。
// 原模板写作 `not(debug_assertions)`，导致 debug 构建总会带一个控制台窗口。
//
// 不影响排障：
//   - 引擎日志本来就落文件（`app\data\chain_test.log` / `audit.log`），UI 也有「审计」页；
//   - 以 `-RedirectStandardOutput` 方式启动时，stdout 重定向仍照常写入文件；
//   - 确实需要实时控制台时：`cargo build --features console`（见 Cargo.toml 的 console feature）。
#![cfg_attr(all(windows, not(feature = "console")), windows_subsystem = "windows")]

fn main() {
    mica_app_lib::run()
}
