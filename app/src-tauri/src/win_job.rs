//! Windows Job Object：MCP 子进程 kill-on-close（发布清单 R06 兜底）。
//!
//! 目标：应用进程无论正常退出、崩溃还是被强杀，OS 都会关闭 Job 句柄，
//! 从而终止加入该 Job 的所有子进程（含 npx 派生的 node）。
//!
//! 实现方式：创建一次 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job，句柄保存在进程内
//! `OnceLock`（进程退出时自动关闭）；每个 MCP 子进程 pid 在 spawn 后加入。
//! 若子进程已在别的 Job 且不允许嵌套，加入会失败 —— 只记日志，不阻断 MCP。

#[cfg(windows)]
pub fn assign_kill_on_close(pid: u32) -> Result<(), String> {
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    static JOB: OnceLock<isize> = OnceLock::new();

    let job = *JOB.get_or_init(|| unsafe {
        match CreateJobObjectW(None, PCWSTR::null()) {
            Ok(h) => {
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .is_ok();
                if ok {
                    h.0 as isize
                } else {
                    0
                }
            }
            Err(_) => 0,
        }
    });
    if job == 0 {
        return Err("创建 Job Object 失败".to_string());
    }

    unsafe {
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)
            .map_err(|e| format!("OpenProcess({pid}) 失败：{e}"))?;
        let res = AssignProcessToJobObject(HANDLE(job as *mut _), process);
        let _ = CloseHandle(process);
        res.map_err(|e| format!("AssignProcessToJobObject({pid}) 失败：{e}"))
    }
}

#[cfg(not(windows))]
pub fn assign_kill_on_close(_pid: u32) -> Result<(), String> {
    Ok(())
}
