; EAgent NSIS 安装钩子：升级/覆盖安装/卸载前终止旧进程。
; 进程树由 Windows Job Object（win_job.rs）兜底；这里先显式杀一次，
; 避免安装器覆盖 exe 时被文件占用（os error 5 的发布场景）。

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Stopping running EAgent processes..."
  nsExec::ExecToLog 'taskkill /F /IM eagent-app.exe /T'
  Sleep 800
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping running EAgent processes..."
  nsExec::ExecToLog 'taskkill /F /IM eagent-app.exe /T'
  Sleep 500
!macroend
