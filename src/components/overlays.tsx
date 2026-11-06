import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../lib/backend";
import type { Profile } from "../types";

export interface ToastMsg {
  id: number;
  kind: "error" | "info";
  title: string;
  message: string;
  requestId?: string | null;
}

export interface CmdkAction {
  icon: string;
  cat: string;
  label: string;
  hint?: string;
  color?: string;
  run: () => void;
}

/** 通用对话框外壳：role=dialog + aria-modal + Esc 关闭 + 打开时焦点移入 */
export function Dialog(props: { onClose: () => void; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    el?.querySelector<HTMLElement>("input, select, textarea, button")?.focus();
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [props.onClose]);
  return (
    <div className="dialog-wrap" onMouseDown={(e) => e.target === e.currentTarget && props.onClose()}>
      <div className="dialog" role="dialog" aria-modal="true" ref={ref}>
        {props.children}
      </div>
    </div>
  );
}

export function Cmdk(props: {
  actions: CmdkAction[];
  onClose: () => void;
}) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);

  const items = useMemo(() => {
    const s = q.trim().toLowerCase();
    if (!s) return props.actions;
    return props.actions.filter(
      (a) => a.label.toLowerCase().includes(s) || a.cat.toLowerCase().includes(s),
    );
  }, [q, props.actions]);

  useEffect(() => setSel(0), [q]);
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
      if (e.key === "ArrowDown") { e.preventDefault(); setSel((s) => (s + 1) % Math.max(items.length, 1)); }
      if (e.key === "ArrowUp") { e.preventDefault(); setSel((s) => (s - 1 + items.length) % Math.max(items.length, 1)); }
      if (e.key === "Enter" && items[sel]) { items[sel].run(); props.onClose(); }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [items, sel, props]);

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && props.onClose()}>
      <div className="cmdk" role="dialog" aria-modal="true" aria-label="命令面板">
        <input
          autoFocus
          aria-label="搜索命令"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="输入命令、集群名、桶名或 s3:// 前缀…"
        />
        <div className="cmdk-list" role="listbox" aria-label="命令结果">
          {items.map((a, i) => (
            <div
              key={a.label}
              role="option"
              aria-selected={i === sel}
              tabIndex={0}
              className={`cmdk-item ${i === sel ? "sel" : ""}`}
              onMouseEnter={() => setSel(i)}
              onClick={() => { a.run(); props.onClose(); }}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") { e.preventDefault(); a.run(); props.onClose(); }
              }}
            >
              <span style={{ color: a.color ?? "var(--ds-fg-3)", width: 18, textAlign: "center" }}>{a.icon}</span>
              <span>{a.label}</span>
              <span className="cat">{a.cat}</span>
              {a.hint && <span className="hint">{a.hint}</span>}
            </div>
          ))}
          {items.length === 0 && (
            <div style={{ padding: 24, textAlign: "center", color: "var(--ds-placeholder)", fontSize: 13 }}>
              无匹配结果
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

export function Toasts(props: { toasts: ToastMsg[]; onDismiss: (id: number) => void }) {
  return (
    <div className="toast-wrap" role="alert" aria-live="polite">
      {props.toasts.map((t) => (
        <div
          className="toast"
          key={t.id}
          role="alert"
          tabIndex={0}
          onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") props.onDismiss(t.id); }}
          onClick={() => props.onDismiss(t.id)}
        >
          <div className="t" style={{ color: t.kind === "error" ? "var(--ds-danger)" : "var(--ds-fg)" }}>
            {t.kind === "error" ? "⚠" : "✓"} {t.title}
          </div>
          <div className="m">{t.message}</div>
          {t.requestId && (
            <div className="rid">
              RequestID: {t.requestId}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

export function useToast() {
  const [toasts, setToasts] = useState<ToastMsg[]>([]);
  const timers = useRef<number[]>([]);
  useEffect(() => () => {
    timers.current.forEach((t) => clearTimeout(t));
    timers.current = [];
  }, []);
  const push = useCallback((t: Omit<ToastMsg, "id">) => {
    const id = Date.now() + Math.random();
    setToasts((ts) => [...ts, { ...t, id }]);
    timers.current.push(
      window.setTimeout(() => setToasts((ts) => ts.filter((x) => x.id !== id)), 5000),
    );
  }, []);
  const dismiss = useCallback((id: number) => {
    setToasts((ts) => ts.filter((x) => x.id !== id));
  }, []);
  const pushS3Error = useCallback(
    (profile: string, e: unknown) => {
      const err = e as { code?: string; message?: string; requestId?: string };
      push({
        kind: "error",
        title: `${err.code ?? "Error"} — ${profile}`,
        message: err.message ?? String(e),
        requestId: err.requestId ?? null,
      });
    },
    [push],
  );
  return { toasts, push, dismiss, pushS3Error };
}

/** 从集群 Profile 列表构建 ⌘K 动作（切集群只 setActiveId，加载由 App 的 effect 统一负责） */
export function buildClusterActions(profiles: Profile[], onSwitch: (id: string) => void): CmdkAction[] {
  return profiles.map((p, i) => ({
    icon: "▣",
    cat: "集群",
    label: `切换到 ${p.name}`,
    hint: `Ctrl+${i + 1}`,
    color: `var(--cluster-${p.color})`,
    run: () => onSwitch(p.id),
  }));
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export const presignAndCopy = async (
  profileId: string,
  bucket: string,
  key: string,
  expiresSec: number,
): Promise<boolean> => {
  const url = await api.presignGet(profileId, bucket, key, expiresSec);
  return copyText(url);
};
