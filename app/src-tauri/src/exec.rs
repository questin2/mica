//! PowerShell 执行：隐藏窗口 / 超时 / 输出上限 / 进程树回收。
//!
//! 拆分自原单文件 `lib.rs`（可扩展性分析报告 v2 · S1）。
//! `use super::*` 继承根模块的导入与共享项，避免重复声明。

use super::*;
// `creation_flags`（CREATE_NO_WINDOW）来自 Windows 的 CommandExt
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ---------- PowerShell 执行（超时/输出上限/隐藏窗口） ----------

/// **终止整个进程树**（`taskkill /T /F`）。
///
/// 存在的理由（原 E-10 的问题）：脚本里 `Start-Process` / `cmd /c` / `node` / `python`
/// 派生的孙子进程**不会随父进程终止**，只杀 powershell.exe 会留下孤儿。
///
/// 抽成独立函数是因为它现在有两个调用方：
/// - 本模块的 PowerShell 超时路径；
/// - `mcp_bridge` 的超时/崩溃清理路径（MCP server 常是 `npx → node` 两层）。
///
/// 注意：**必须用 pid**，绝不按进程名杀（会误杀用户的其他 node/npx 进程）。
pub(crate) fn kill_process_tree(pid: u32) -> Result<(), String> {
    let out = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("taskkill 启动失败：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "taskkill 失败（{}）：{}",
            out.status.code().unwrap_or(-1),
            decode_bytes(&out.stderr).trim()
        ))
    }
}

pub(crate) fn decode_bytes(b: &[u8]) -> String {
    // 优先检测 BOM：EF BB BF = UTF-8，FF FE = UTF-16LE，FE FF = UTF-16BE
    if b.len() >= 3 && b[0] == 0xEF && b[1] == 0xBB && b[2] == 0xBF {
        return encoding_rs::UTF_8.decode(&b[3..]).0.to_string();
    }
    if b.len() >= 2 && b[0] == 0xFF && b[1] == 0xFE {
        return encoding_rs::UTF_16LE.decode(&b[2..]).0.to_string();
    }
    if b.len() >= 2 && b[0] == 0xFE && b[1] == 0xFF {
        return encoding_rs::UTF_16BE.decode(&b[2..]).0.to_string();
    }
    // 无 BOM：先尝试 UTF-8 容错解码（非法字节替换为 U+FFFD，不再整体失败回退）
    let (cow, _, had_errors) = encoding_rs::UTF_8.decode(b);
    if !had_errors {
        return cow.to_string();
    }
    // UTF-8 有非法字节 → 按 GBK 解（PS5.1 老输出或中文系统编码）
    encoding_rs::GBK.decode(b).0.to_string()
}

pub(crate) fn execute_powershell(workdir: &str, code: &str) -> Result<String, String> {

    std::fs::create_dir_all(tmp_dir()).map_err(|e| e.to_string())?;
    let script_path = format!(r"{}\tool_{}_{}.ps1", tmp_dir(), now_ms(), SEQ.fetch_add(1, Ordering::Relaxed));
    // 脚本头：强制 UTF-8 输出 + 遇错即停（防止语法错误后继续执行导致级联破坏）
    let mut script = String::from(
        "$ErrorActionPreference='Stop';[Console]::OutputEncoding=[Text.Encoding]::UTF8;$OutputEncoding=[Text.Encoding]::UTF8;\n",
    );
    script.push_str(code);
    let mut bytes = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM：PS5.1 无 BOM 时按 ANSI 读源码，中文会乱码
    bytes.extend(script.as_bytes());
    std::fs::write(&script_path, bytes).map_err(|e| e.to_string())?;

    // 清理闭包：确保临时脚本文件在所有退出路径都被删除
    let cleanup = || { let _ = std::fs::remove_file(&script_path); };

    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script_path]);
    if !workdir.is_empty() && Path::new(workdir).exists() {
        cmd.current_dir(workdir);
    }
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW：不弹命令行窗口

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { cleanup(); return Err(format!("spawn failed: {e}")); }
    };
    let child_pid = child.id();
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });

    // 【S3】超时/上限来自 config/agent.json（运行时读取）
    let cfg = config::agent_cfg();
    let deadline = SystemTime::now() + Duration::from_secs(cfg.tool_timeout_sec);
    let mut timed_out = false;
    let status;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => {
                if SystemTime::now() > deadline {
                    // 【E-10】超时必须终止整个进程树，不能只杀 powershell.exe。
                    // 脚本中 Start-Process / cmd /c / node / python 等派生的孙子进程
                    // 不会随父进程终止，必须用 taskkill /T（terminate tree）。
                    let _ = kill_process_tree(child_pid);
                    timed_out = true;
                    status = None;
                    break;
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => { cleanup(); return Err(format!("wait failed: {e}")); }
        }
    }
    let out = out_handle.join().unwrap_or_default();
    let err = err_handle.join().unwrap_or_default();
    let mut combined = decode_bytes(&out);
    let err_s = decode_bytes(&err);
    if !err_s.trim().is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str("[stderr] ");
        combined.push_str(&err_s);
    }
    if timed_out {
        combined.push_str(&format!("\n[超时终止：超过 {}s，进程树已强制结束]", cfg.tool_timeout_sec));
    } else if let Some(s) = status {
        if !s.success() {
            combined.push_str(&format!("\n[退出码: {}]", s.code().unwrap_or(-1)));
        }
    }
    let total = combined.chars().count();
    let cap = cfg.tool_output_cap;
    if total > cap {
        let head: String = combined.chars().take(cap).collect();
        combined = format!("{head}\n[结果已截断：原 {total} 字符，仅回传前 {cap}；如需完整内容请改用文件批处理]");
    }
    cleanup(); // 【E-11】执行完毕删除临时脚本，防止磁盘累积和敏感信息留盘
    Ok(combined)
}
