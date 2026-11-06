import { useState } from "react";
import { IconClose, IconDownload, IconUpload } from "./icons";
import type { TransferEvent } from "../lib/backend";
import { formatSize } from "../lib/backend";

/** 传输条：消费 Rust 端 upload_file/download_file 推送的 transfer://event */
export default function TransferBar(props: { tasks: TransferEvent[]; onClearTask: (taskId: string) => void }) {
  const [open, setOpen] = useState(true);
  const running = props.tasks.filter((t) => t.state === "running");
  const cur = running[0];
  const toggle = () => setOpen(!open);
  return (
    <div className={`transfer-bar ${open ? "open" : ""}`}>
      <div
        className="tb-head"
        role="button"
        tabIndex={0}
        aria-expanded={open}
        onClick={toggle}
        onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggle(); } }}
      >
        {running.some((t) => t.kind === "upload") ? <IconUpload size={15} /> : <IconDownload size={15} />}
        <span className="tb-title">
          传输{props.tasks.length > 0 ? ` · ${running.length} 个任务进行中 / 共 ${props.tasks.length}` : " · 空闲"}
        </span>
        {cur && cur.total > 0 && (
          <div
            className="tb-progress"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round((cur.done / cur.total) * 100)}
          >
            <div style={{ width: `${Math.round((cur.done / cur.total) * 100)}%` }} />
          </div>
        )}
        {cur && (
          <span className="tb-sub">
            {formatSize(cur.done)} / {formatSize(cur.total)}
          </span>
        )}
      </div>
      <div className="tb-body" onClick={(e) => e.stopPropagation()}>
        {props.tasks.length === 0 && (
          <div className="empty-task">暂无传输任务。上传 / 下载任务将显示于此。</div>
        )}
        {props.tasks.map((t) => {
          const pct = t.total > 0 ? Math.round((t.done / t.total) * 100) : 0;
          const finished = t.state === "done" || t.state === "failed";
          return (
            <div className="task" key={t.taskId}>
              <div className="task-head">
                <span className={`chip ${t.kind === "upload" ? "brand" : "neutral"}`}>
                  {t.kind === "upload" ? "上传" : "下载"}
                </span>
                <span className="task-name">
                  {t.kind === "upload" ? `${t.localPath} → ${t.cluster}/${t.bucket}/${t.key}` : `${t.cluster}/${t.bucket}/${t.key} → ${t.localPath}`}
                </span>
                <span className={`chip ${t.state === "done" ? "success" : t.state === "failed" ? "danger" : "progress"}`}>
                  {t.state === "done" ? "已完成" : t.state === "failed" ? `失败：${t.error?.code ?? ""}` : t.total > 0 ? `${pct}%` : "…"}
                </span>
                {finished && (
                  <button className="icon-btn sm" title="从列表清除" onClick={() => props.onClearTask(t.taskId)}>
                    <IconClose size={13} />
                  </button>
                )}
              </div>
              {t.state === "running" && t.total > 0 && (
                <div
                  className="tb-progress"
                  style={{ maxWidth: "100%", marginTop: 8 }}
                  role="progressbar"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={pct}
                  aria-label={`${t.kind === "upload" ? "上传" : "下载"} ${t.key}`}
                >
                  <div style={{ width: `${pct}%` }} />
                </div>
              )}
              {t.state === "failed" && t.error && (
                <div className="task-log" style={{ color: "var(--ds-danger)" }}>{t.error.message}</div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
