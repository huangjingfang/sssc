import type { KeyboardEvent } from "react";
import type { ConnectionStatus, Profile } from "../types";
import { IconMenu, IconMoon, IconSun, IconPlus, IconSearch, IconTrash, IconEdit, IconLogo } from "./icons";

const activateOnKey = (fn: () => void) => (e: KeyboardEvent) => {
  if (e.key === "Enter" || e.key === " ") { e.preventDefault(); fn(); }
};

export default function Sidebar(props: {
  profiles: Profile[];
  statuses: Record<string, ConnectionStatus | undefined>;
  activeId: string | null;
  collapsed: boolean;
  dark: boolean;
  onSwitch: (id: string) => void;
  onEdit: (p: Profile) => void;
  onDelete: (p: Profile) => void;
  onAdd: () => void;
  onToggleCollapse: () => void;
  onToggleTheme: () => void;
  onOpenCmdk: () => void;
}) {
  const { profiles, statuses, activeId } = props;
  return (
    <aside className={`sidebar ${props.collapsed ? "collapsed" : ""}`}>
      <div className="side-head">
        <div
          className={`logo ${props.collapsed ? "clickable" : ""}`}
          title={props.collapsed ? "展开侧栏" : undefined}
          onClick={props.collapsed ? props.onToggleCollapse : undefined}
        >
          <IconLogo size={28} />
        </div>
        <span className="title side-label">SSSC</span>
        {!props.collapsed && (
          <button className="icon-btn" title="折叠侧栏" onClick={props.onToggleCollapse}>
            <IconMenu />
          </button>
        )}
      </div>
      <button className="cmdk-trigger" onClick={props.onOpenCmdk}>
        <IconSearch size={14} />
        <span className="trigger-text">搜索对象、切换集群…</span>
        <span className="kbd">⌘K</span>
      </button>
      <div className="cluster-group" role="list">
        <div className="side-label">集群 · {profiles.length}</div>
        {profiles.map((p) => {
          const st = statuses[p.id];
          return (
            <div
              key={p.id}
              role="button"
              tabIndex={0}
              aria-pressed={p.id === activeId}
              className={`cluster-item ${p.id === activeId ? "active" : ""}`}
              onClick={() => props.onSwitch(p.id)}
              onKeyDown={activateOnKey(() => props.onSwitch(p.id))}
            >
              <div className="cluster-color" style={{ background: `var(--cluster-${p.color})` }} />
              <div className="cluster-mini" title={p.name}
                style={{ background: `var(--cluster-${p.color})` }}>
                {p.name.slice(0, 1).toUpperCase()}
              </div>
              <div className="cluster-body">
                <div className="cluster-name">
                  {p.name}
                  {!p.enabled && <span className="cluster-tag">禁用</span>}
                </div>
                <div className="cluster-meta">
                  <span className={`dot ${st ? (st.ok ? "on" : "off") : "off"}`} />
                  {st
                    ? st.ok
                      ? `已连接 · ${st.latencyMs ?? "—"}ms · ${st.bucketCount ?? "—"} 桶`
                      : `${st.error?.code ?? "连接失败"}`
                    : "未测试"
                  }
                </div>
              </div>
              <button
                className="icon-btn sm cluster-del"
                title="编辑集群"
                onClick={(ev) => { ev.stopPropagation(); props.onEdit(p); }}
                onKeyDown={(ev) => ev.stopPropagation()}
              >
                <IconEdit size={13} />
              </button>
              <button
                className="icon-btn sm cluster-del"
                title="删除集群"
                onClick={(e) => { e.stopPropagation(); props.onDelete(p); }}
                onKeyDown={(e) => e.stopPropagation()}
              >
                <IconTrash size={13} />
              </button>
            </div>
          );
        })}
        <button
          className="foot-item"
          style={{ marginTop: 6, color: "var(--ds-color-brand-medium)" }}
          onClick={props.onAdd}
        >
          <IconPlus />
          <span className="side-foot-text">添加集群</span>
        </button>
      </div>
      <div className="side-foot">
        <button className="foot-item" onClick={props.onToggleTheme} aria-pressed={props.dark}>
          {props.dark ? <IconSun /> : <IconMoon />}
          <span className="side-foot-text">{props.dark ? "浅色模式" : "深色模式"}</span>
        </button>
      </div>
    </aside>
  );
}
