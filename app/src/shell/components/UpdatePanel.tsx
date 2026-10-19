// 自动更新（策划案发布硬化 · Tauri updater / minisign）
//
// 更新源：tauri.conf.json → plugins.updater.endpoints（默认 GitHub Releases 的 latest.json）。
// 签名校验由 updater 插件在 Rust 侧完成；本组件只负责触发与展示进度。
import { useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { Icon } from "../icons";

type Phase = "idle" | "checking" | "available" | "downloading" | "installing" | "done";

export function UpdatePanel() {
  const [current, setCurrent] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [status, setStatus] = useState("");
  const [progress, setProgress] = useState<{ got: number; total: number | null }>({ got: 0, total: null });
  const [err, setErr] = useState("");

  useEffect(() => {
    getVersion().then(setCurrent).catch(() => setCurrent("未知"));
  }, []);

  const doCheck = useCallback(async () => {
    setPhase("checking");
    setErr("");
    setStatus("正在检查更新…");
    setUpdate(null);
    try {
      const u = await check();
      if (u) {
        setUpdate(u);
        setPhase("available");
        setStatus(`发现新版本 ${u.version}（当前 ${u.currentVersion}）`);
      } else {
        setPhase("idle");
        setStatus("已是最新版本");
      }
    } catch (e) {
      setPhase("idle");
      setStatus("");
      setErr(`检查失败：${String(e)}（更新源尚未发布或网络不可达）`);
    }
  }, []);

  const doInstall = useCallback(async () => {
    if (!update) return;
    setPhase("downloading");
    setErr("");
    setProgress({ got: 0, total: null });
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          setProgress({ got: 0, total: event.data.contentLength ?? null });
          setStatus("开始下载…");
        } else if (event.event === "Progress") {
          setProgress((p) => ({ ...p, got: p.got + event.data.chunkLength }));
        } else if (event.event === "Finished") {
          setPhase("installing");
          setStatus("下载完成，正在安装…");
        }
      });
      setPhase("done");
      setStatus("安装完成，正在重启…");
      await relaunch();
    } catch (e) {
      setPhase("available");
      setErr(`安装失败：${String(e)}`);
    }
  }, [update]);

  const pct =
    progress.total && progress.total > 0
      ? Math.min(100, Math.round((progress.got / progress.total) * 100))
      : null;

  return (
    <div className="section">
      <h2 className="section-title">更新</h2>
      <div className="card">
        <div className="field">
          <div className="field-label">
            <b>当前版本</b>
            <span>更新包必须通过 minisign 校验；签名失败会拒绝安装</span>
          </div>
          <div className="field-control">
            <span className="badge mono">{current || "读取中…"}</span>
            <button
              className="btn"
              disabled={phase === "checking" || phase === "downloading" || phase === "installing"}
              onClick={() => void doCheck()}
            >
              <Icon name="refresh" size={14} />
              {phase === "checking" ? "检查中…" : "检查更新"}
            </button>
            {update && phase !== "done" && (
              <button className="btn primary" onClick={() => void doInstall()}>
                <Icon name="download" size={14} />
                下载并安装 {update.version}
              </button>
            )}
          </div>
        </div>

        {status && <div className="pc-desc">{status}</div>}
        {err && <div className="pc-err">{err}</div>}

        {update?.body && (
          <div className="tc-hint" style={{ whiteSpace: "pre-wrap", marginTop: "var(--s-2)" }}>
            {update.body}
          </div>
        )}

        {(phase === "downloading" || phase === "installing") && (
          <div style={{ marginTop: "var(--s-2)" }}>
            <div
              style={{
                height: 6,
                borderRadius: 3,
                background: "var(--surface-3)",
                overflow: "hidden",
              }}
            >
              <div
                style={{
                  width: pct === null ? "20%" : `${pct}%`,
                  height: "100%",
                  background: "var(--accent)",
                  transition: "width .25s ease",
                }}
              />
            </div>
            <div className="tc-hint">
              {pct === null ? "下载中…" : `${pct}%`}
              {progress.total ? ` · ${fmtBytes(progress.got)} / ${fmtBytes(progress.total)}` : ""}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function fmtBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${n} B`;
}
