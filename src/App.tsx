import { useCallback, useEffect, useRef, useState } from "react";
import { api, pickFiles, pickSavePath, onTransferEvent, onBatchProgress, safeDecode, type TransferEvent, type BatchProgress } from "./lib/backend";
import type { Bucket, ConnectionStatus, ListResult, Profile, S3Object } from "./types";
import Sidebar from "./components/Sidebar";
import AddClusterDialog from "./components/AddClusterDialog";
import ObjectTable from "./components/ObjectTable";
import DetailPanel from "./components/DetailPanel";
import TransferBar from "./components/TransferBar";
import { Cmdk, Toasts, useToast, CmdkAction, buildClusterActions, Dialog } from "./components/overlays";
import { IconPlus, IconRefresh, IconUpload, IconFolder, IconSpinner, IconLink } from "./components/icons";
import ErrorBoundary from "./components/ErrorBoundary";

const EMPTY_LIST: ListResult = { objects: [], prefixes: [], nextToken: null, keyCount: 0 };

function initialDark(): boolean {
  try {
    const saved = localStorage.getItem("sssc-theme");
    if (saved === "dark") return true;
    if (saved === "light") return false;
  } catch { /* localStorage 不可用时忽略 */ }
  return typeof window !== "undefined" && !!window.matchMedia?.("(prefers-color-scheme: dark)").matches;
}

export default function App() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [statuses, setStatuses] = useState<Record<string, ConnectionStatus>>({});
  const [activeId, setActiveId] = useState<string | null>(null);
  const [buckets, setBuckets] = useState<Bucket[] | null>(null);
  const [bucket, setBucket] = useState<string | null>(null);
  const [prefix, setPrefix] = useState("");
  const [list, setList] = useState<ListResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [selected, setSelected] = useState<S3Object | null>(null);
  const [cmdkOpen, setCmdkOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<Profile | null>(null);
  const [collapsed, setCollapsed] = useState(false);
  const [dark, setDark] = useState(initialDark);
  const [deleteTarget, setDeleteTarget] = useState<Profile | null>(null);
  const [deleteObjTarget, setDeleteObjTarget] = useState<S3Object | null>(null);
  /** 单项删除 loading（对象/集群/桶各自的确认对话框按钮态） */
  const [deletingObj, setDeletingObj] = useState(false);
  const [deletingCluster, setDeletingCluster] = useState(false);
  const [deletingBucket, setDeletingBucket] = useState(false);
  /** 多选删除：待确认删除的 key 列表（对话框确认前） */
  const [deleteManyTarget, setDeleteManyTarget] = useState<string[] | null>(null);
  const [deletingMany, setDeletingMany] = useState(false);
  /** 对象多选勾选（当前桶+前缀视图内） */
  const [checkedKeys, setCheckedKeys] = useState<string[]>([]);
  const [tasks, setTasks] = useState<TransferEvent[]>([]);
  const [newBucketOpen, setNewBucketOpen] = useState(false);
  const [newBucketName, setNewBucketName] = useState("");
  const [newFolderOpen, setNewFolderOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  const [bucketDeleteTarget, setBucketDeleteTarget] = useState<string | null>(null);
  const { toasts, push, dismiss, pushS3Error } = useToast();

  /* 请求序号：过期响应（竞态）丢弃 */
  const reqSeq = useRef(0);
  const active = profiles.find((p) => p.id === activeId) ?? null;
  /** 用户主动断开连接的集群（本地状态；ref 供 effect 无依赖读取） */
  const disconnectedRef = useRef<Set<string>>(new Set());
  const activeDisconnected = !!statuses[activeId ?? ""]?.disconnected;

  /* 深浅色主题持久化 */
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    try { localStorage.setItem("sssc-theme", dark ? "dark" : "light"); } catch { /* 忽略 */ }
  }, [dark]);

  /* ---------- 加载 ---------- */
  const loadObjects = useCallback(async (pid: string, b: string, p: string) => {
    const seq = ++reqSeq.current;
    setLoading(true);
    setSelected(null);
    setCheckedKeys([]);
    try {
      const res = await api.listObjects(pid, b, p, null);
      if (reqSeq.current !== seq) return; // 过期响应丢弃
      setList(res);
    } catch (e) {
      if (reqSeq.current !== seq) return;
      setList(EMPTY_LIST);
      pushS3Error(`list_objects ${b}`, e);
    } finally {
      if (reqSeq.current === seq) setLoading(false);
    }
  }, [pushS3Error]);

  /** “加载更多”：以当前 nextToken 作为 continuationToken 追加下一页 */
  const loadMoreObjects = useCallback(async () => {
    const token = list?.nextToken;
    if (!activeId || !bucket || !token || loading || loadingMore) return;
    const seq = ++reqSeq.current;
    setLoadingMore(true);
    try {
      const res = await api.listObjects(activeId, bucket, prefix, null, token);
      if (reqSeq.current !== seq) return;
      setList((prev) => {
        // 仅当分页游标未变时追加，避免与重置竞态
        if (!prev || prev.nextToken !== token) return prev;
        return {
          objects: [...prev.objects, ...res.objects],
          prefixes: [...prev.prefixes, ...res.prefixes],
          nextToken: res.nextToken,
          keyCount: prev.keyCount + res.keyCount,
        };
      });
    } catch (e) {
      if (reqSeq.current === seq) pushS3Error("list_objects 加载更多", e);
    } finally {
      if (reqSeq.current === seq) setLoadingMore(false);
    }
  }, [activeId, bucket, prefix, list, loading, loadingMore, pushS3Error]);

  const loadBuckets = useCallback(async (pid: string) => {
    disconnectedRef.current.delete(pid); // 主动刷新/重连视为恢复连接
    const seq = ++reqSeq.current;
    setLoading(true);
    setBucket(null);
    setPrefix("");
    setList(null);
    setSelected(null);
    try {
      const started = performance.now();
      const bs = await api.listBuckets(pid);
      if (reqSeq.current !== seq) return;
      setBuckets(bs);
      // 每次加载实测：延迟 = list_buckets 请求耗时，桶数 = 本次返回数量
      const latencyMs = Math.round(performance.now() - started);
      setStatuses((s) => ({
        ...s,
        [pid]: { ok: true, latencyMs, bucketCount: bs.length, error: null },
      }));
    } catch (e) {
      if (reqSeq.current !== seq) return;
      setBuckets([]);
      setStatuses((s) => ({ ...s, [pid]: { ok: false, latencyMs: null, bucketCount: null, error: e as never } }));
      pushS3Error("list_buckets", e);
    } finally {
      if (reqSeq.current === seq) setLoading(false);
    }
  }, [pushS3Error]);

  useEffect(() => {
    (async () => {
      const ps = await api.listProfiles();
      setProfiles(ps);
      if (ps[0]) setActiveId(ps[0].id);
    })();
  }, []);

  useEffect(() => {
    if (!activeId) return;
    if (disconnectedRef.current.has(activeId)) {
      // 已断开的集群：切换/重挂载时不自动重连，仅清空旧数据
      setBuckets(null); setBucket(null); setPrefix(""); setList(null);
      return;
    }
    loadBuckets(activeId);
  }, [activeId, loadBuckets]);

  useEffect(() => {
    if (activeId && bucket) loadObjects(activeId, bucket, prefix);
  }, [activeId, bucket, prefix, loadObjects]);

  /** 批量删除进度（事件驱动） */
  const [batchProg, setBatchProg] = useState<BatchProgress | null>(null);
  /* ---------- 传输事件 → 传输条任务列表（订阅一次；StrictMode 下不泄漏） ---------- */
  const listCtx = useRef({ activeId, bucket, prefix });
  listCtx.current = { activeId, bucket, prefix };
  const reloadRef = useRef(loadObjects);
  reloadRef.current = loadObjects;
  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | undefined;
    onTransferEvent((ev) => {
      setTasks((ts) => {
        const i = ts.findIndex((t) => t.taskId === ev.taskId);
        if (i >= 0) { const c = [...ts]; c[i] = ev; return c; }
        return [...ts, ev];
      });
      if (ev.state === "done") {
        const { activeId: pid, bucket: b, prefix: p } = listCtx.current;
        if (pid && b) reloadRef.current(pid, b, p);
      }
    }).then((u) => {
      // cleanup 已执行（如 StrictMode 双挂载）时立即退订，避免泄漏
      if (cancelled) u();
      else un = u;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | undefined;
    onBatchProgress((ev) => {
      setBatchProg((prev) => (prev && prev.batchId !== ev.batchId && ev.phase !== "listing" ? prev : ev));
    }).then((u) => {
      if (cancelled) u();
      else un = u;
    });
    return () => { cancelled = true; un?.(); };
  }, []);

  /* ---------- 交互 ---------- */
  const switchCluster = (id: string) => {
    // 同一批次内清空桶/前缀/列表：否则 loadObjects effect 会在本次提交里带着
    // 旧集群的 bucket + 新集群的 activeId 发请求（跨集群错配 → NoSuchBucket）
    if (id === activeId) return;
    setActiveId(id);
    setBucket(null);
    setPrefix("");
    setList(null);
    setBuckets(null);
    setSelected(null);
    setCheckedKeys([]);
  };

  /** 断开集群连接：仅本地状态，不产生任何远端请求；清空已加载数据 */
  const disconnectCluster = (p: Profile) => {
    disconnectedRef.current.add(p.id);
    setStatuses((s) => ({
      ...s,
      [p.id]: { ok: false, latencyMs: null, bucketCount: null, error: null, disconnected: true },
    }));
    if (p.id === activeId) {
      setBuckets(null); setBucket(null); setPrefix(""); setList(null);
      setSelected(null); setCheckedKeys([]);
    }
    push({ kind: "info", title: "已断开连接", message: `${p.name} 已断开（仅本地状态，不影响远端）` });
  };

  /** 重新连接：重新探活（ListBuckets）并恢复桶列表 */
  const reconnectCluster = async (p: Profile) => {
    if (p.id === activeId) {
      await loadBuckets(p.id);
    } else {
      // 非当前集群：只复位状态为“未测试”，切换过去时 effect 会自动探活
      disconnectedRef.current.delete(p.id);
      setStatuses((s) => {
        const { [p.id]: _removed, ...rest } = s;
        return rest as typeof s;
      });
    }
  };

  const confirmDelete = async () => {
    const target = deleteTarget;
    if (!target) return;
    setDeletingCluster(true);
    try {
      await api.deleteProfile(target.id);
    } catch (e) {
      pushS3Error("删除集群失败", e);
      return;
    } finally {
      setDeletingCluster(false);
    }
    // 删除成功后再更新本地状态（副作用不放 setState updater 内）
    setDeleteTarget(null);
    const rest = profiles.filter((p) => p.id !== target.id);
    setProfiles(rest);
    if (activeId === target.id) {
      setActiveId(rest[0]?.id ?? null);
      setBuckets(null);
      setBucket(null);
    }
    push({ kind: "info", title: "已删除集群", message: `${target.name} 已从本机移除（密钥同步清除）` });
  };

  const selectBucket = (b: string) => { setBucket(b); setPrefix(""); };
  const enterPrefix = (p: string) => { setPrefix(p); };
  const goUp = () => {
    if (!prefix) { setBucket(null); return; }
    const parts = prefix.split("/").filter(Boolean);
    parts.pop();
    setPrefix(parts.length ? parts.join("/") + "/" : "");
  };

  const crumbs = bucket ? prefix.split("/").filter(Boolean) : [];

  const cmdkActions: CmdkAction[] = [
    ...buildClusterActions(profiles, switchCluster),
    ...buckets?.map((b) => ({
      icon: "⌂", cat: "桶", label: `s3://${active?.name ?? ""}/${safeDecode(b.name)}/`, run: () => selectBucket(b.name),
    })) ?? [],
    { icon: "＋", cat: "集群", label: "添加集群…", color: "var(--ds-color-brand-medium)", run: () => setAddOpen(true) },
    ...(active ? [{
      icon: activeDisconnected ? "⇄" : "⤫",
      cat: "集群",
      label: activeDisconnected ? `重新连接 ${active.name}` : `断开连接 ${active.name}`,
      run: () => { activeDisconnected ? void reconnectCluster(active) : disconnectCluster(active); },
    }] : []),
    ...(activeId && bucket ? [{
      icon: "↑", cat: "操作", label: `上传文件到 ${safeDecode(prefix) || "根目录"}`, run: () => { void doUpload(); },
    }] : []),
  ];

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); setCmdkOpen(true); }
      if (e.ctrlKey && /^[1-9]$/.test(e.key)) {
        const p = profiles[Number(e.key) - 1];
        if (p) {
          // ⌘K 打开时切换集群同时关闭面板，避免双重加载
          setCmdkOpen(false);
          switchCluster(p.id);
        }
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [profiles]);

  const doUpload = async () => {
    if (!activeId || !bucket) return;
    const files = await pickFiles();
    if (!files?.length) return;
    for (const f of files) {
      // Windows 本地路径分隔符是 \（如 C:\folder1\a.txt），需同时按 /\ 切分取文件名
      const base = f.split(/[\\/]/).pop() ?? f;
      const key = prefix + base;
      api.uploadFile(activeId, bucket, key, f).catch((e) => pushS3Error("上传失败", e));
    }
  };

  const doDownload = async (key: string) => {
    if (!activeId || !bucket) return;
    const base = key.split("/").pop() ?? key;
    const save = await pickSavePath(base);
    if (!save) return;
    api.downloadFile(activeId, bucket, key, save)
      .then(() => push({ kind: "info", title: "下载完成", message: `${key} → ${save}` }))
      .catch((e) => pushS3Error("下载失败", e));
    push({ kind: "info", title: "下载已开始", message: `${key} → ${save}` });
  };

  const confirmDeleteObject = async () => {
    const target = deleteObjTarget;
    if (!target || !activeId || !bucket) return;
    setDeletingObj(true);
    try {
      await api.deleteObject(activeId, bucket, target.key);
      setDeleteObjTarget(null);
      if (selected?.key === target.key) setSelected(null);
      if (activeId && bucket) await loadObjects(activeId, bucket, prefix);
      push({ kind: "info", title: "对象已删除", message: `${bucket}/${target.key}` });
    } catch (e) {
      pushS3Error("删除对象失败", e);
    } finally {
      setDeletingObj(false);
    }
  };

  /* ---------- 多选删除 ---------- */
  const toggleCheck = (key: string) => {
    setCheckedKeys((ks) => (ks.includes(key) ? ks.filter((k) => k !== key) : [...ks, key]));
  };
  const toggleAllObjects = () => {
    const keys = [...(list?.prefixes ?? []), ...(list?.objects.map((o) => o.key) ?? [])];
    const allIn = keys.length > 0 && keys.every((k) => checkedKeys.includes(k));
    setCheckedKeys(allIn ? [] : keys);
  };

  const confirmDeleteObjects = async () => {
    const keys = deleteManyTarget;
    if (!keys?.length || !activeId || !bucket) return;
    setDeletingMany(true);
    // 文件与文件夹（前缀）分组，一次后端调用完成：文件 DeleteObjects 批量，文件夹递归展开
    const files = keys.filter((k) => !k.endsWith("/"));
    const folders = keys.filter((k) => k.endsWith("/"));
    const batchId = `del_${Date.now()}`;
    setBatchProg({ batchId, done: 0, total: 0, phase: "listing" });
    try {
      const r = await api.deleteBatch(activeId, bucket, files, folders, batchId);
      setCheckedKeys((ks) => ks.filter((k) => !keys.includes(k)));
      if (selected && keys.some((k) => k === selected.key || selected.key.startsWith(k))) setSelected(null);
      const folderN = folders.length;
      const fileN = files.length;
      const scope = [fileN > 0 ? `${fileN} 个文件` : null, folderN > 0 ? `${folderN} 个文件夹` : null]
        .filter(Boolean).join("、");
      if (r.errorCount === 0) {
        push({
          kind: "info",
          title: "批量删除完成",
          message: `已删除 ${scope}${r.deletedCount > 0 ? `（共 ${r.deletedCount} 个对象）` : ""}`,
        });
      } else {
        const shown = r.failures
          .slice(0, 3)
          .map((f) => `${safeDecode(f.key)}：[${f.code}] ${f.message}`)
          .join("；");
        pushS3Error(
          `批量删除：${r.errorCount} 个对象失败（其余 ${r.deletedCount} 个已删除）`,
          { code: "DeleteObjects", message: shown || "删除失败", requestId: null, httpStatus: null },
        );
      }
    } catch (e) {
      pushS3Error("批量删除失败", e);
    } finally {
      setDeletingMany(false);
      setBatchProg(null);
      setDeleteManyTarget(null);
      if (activeId && bucket) await loadObjects(activeId, bucket, prefix);
      if (folders.length > 0) {
        // SeaweedFS 删除后列表有短暂不一致：延时补刷一次
        window.setTimeout(() => {
          const { activeId: pid, bucket: b, prefix: pp } = listCtx.current;
          if (pid && b) reloadRef.current(pid, b, pp);
        }, 1200);
      }
    }
  };

  const onPresignSuccess = (key: string, expiresSec: number) => {
    const validity = expiresSec >= 86400 ? `${expiresSec / 86400} 天` : `${expiresSec / 3600} 小时`;
    push({ kind: "info", title: "预签名链接已复制", message: `${key}（有效期 ${validity}）` });
  };

  const doCreateBucket = async () => {
    if (!activeId || !newBucketName.trim()) return;
    const name = newBucketName.trim();
    setNewBucketOpen(false);
    setNewBucketName("");
    try {
      await api.createBucket(activeId, name);
      await loadBuckets(activeId);
      push({ kind: "info", title: "建桶成功", message: `s3://${active?.name}/${name}` });
    } catch (e) { pushS3Error("建桶失败", e); }
  };

  const doCreateFolder = async () => {
    if (!activeId || !bucket || !newFolderName.trim()) return;
    const name = newFolderName.trim().replace(/\/+$/, "");
    setNewFolderOpen(false);
    setNewFolderName("");
    try {
      await api.createFolder(activeId, bucket, prefix + name);
      await loadObjects(activeId, bucket, prefix);
      push({ kind: "info", title: "文件夹已创建", message: `${prefix}${name}/` });
    } catch (e) { pushS3Error("创建文件夹失败", e); }
  };

  const doDeleteBucket = async () => {
    if (!activeId || !bucketDeleteTarget) return;
    const name = bucketDeleteTarget;
    setDeletingBucket(true);
    try {
      await api.deleteBucket(activeId, name);
      setBucketDeleteTarget(null);
      if (bucket === name) { setBucket(null); setPrefix(""); }
      await loadBuckets(activeId);
      push({ kind: "info", title: "删桶成功", message: `${name} 已删除` });
    } catch (e) {
      pushS3Error("删桶失败", e);
    } finally {
      setDeletingBucket(false);
    }
  };

  /* ---------- 空状态 ---------- */
  if (profiles.length === 0 && !addOpen) {
    return (
      <div className="app">
        <div className="empty-state" style={{ flex: 1 }}>
          <div className="empty-card">
            <h2>连接你的第一个集群</h2>
            <p>支持 AWS S3 与 MinIO / Ceph / SeaweedFS 等 S3 兼容端点</p>
            <button className="btn btn-primary" style={{ height: 40, padding: "0 24px" }} onClick={() => setAddOpen(true)}>
              <IconPlus size={15} /> 添加集群
            </button>
          </div>
        </div>
        {addOpen && (
          <AddClusterDialog
            onClose={() => setAddOpen(false)}
            onSaved={(p) => { setProfiles([p]); setActiveId(p.id); setAddOpen(false); }}
          />
        )}
      </div>
    );
  }

  return (
    <div className="app">
      <Sidebar
        profiles={profiles}
        statuses={statuses}
        activeId={activeId}
        collapsed={collapsed}
        dark={dark}
        onSwitch={switchCluster}
        onEdit={(p) => setEditTarget(p)}
        onDelete={(p) => setDeleteTarget(p)}
        onDisconnect={disconnectCluster}
        onReconnect={(p) => void reconnectCluster(p)}
        onAdd={() => setAddOpen(true)}
        onToggleCollapse={() => setCollapsed(!collapsed)}
        onToggleTheme={() => setDark(!dark)}
        onOpenCmdk={() => setCmdkOpen(true)}
      />
      <div className="main">
        <ErrorBoundary>
        <div className="cluster-accent" style={{ background: active ? `var(--cluster-${active.color})` : "transparent" }} />
        <div className="toolbar">
          <div className="breadcrumb">
            <button onClick={goUp} title={active?.name}>{active?.name ?? "—"}</button>
            {bucket && <span className="sep">/</span>}
            {bucket && <button onClick={() => { setBucket(null); setPrefix(""); }} title={bucket}>{bucket}</button>}
            {crumbs.map((c, i) => (
              <span key={i} style={{ display: "flex", alignItems: "center", gap: 2 }}>
                <span className="sep">/</span>
                {i === crumbs.length - 1
                  ? <span className="cur mono" title={safeDecode(c)}>{safeDecode(c)}</span>
                  : <button className="mono" title={safeDecode(c)} onClick={() => setPrefix(crumbs.slice(0, i + 1).join("/") + "/")}>{safeDecode(c)}</button>}
              </span>
            ))}
          </div>
          <div className="spacer" />
          {activeDisconnected ? (
            <button className="btn btn-primary" onClick={() => active && reconnectCluster(active)}>
              <IconLink size={14} /> 重新连接
            </button>
          ) : !bucket ? (
            <button className="btn btn-primary" onClick={() => setNewBucketOpen(true)}>
              <IconPlus size={14} /> 新建桶
            </button>
          ) : (
            <>
              <button className="btn btn-secondary" onClick={() => setNewFolderOpen(true)}>
                <IconFolder size={14} /> 新建文件夹
              </button>
              <button className="btn btn-primary" onClick={doUpload}>
                <IconUpload size={14} /> 上传
              </button>
            </>
          )}
          <button className="icon-btn" title="刷新" onClick={() => activeId && (bucket ? loadObjects(activeId, bucket, prefix) : loadBuckets(activeId))}>
            <IconRefresh />
          </button>
        </div>

        <ObjectTable
          loading={loading}
          loadingMore={loadingMore}
          list={list}
          buckets={buckets}
          bucket={bucket}
          prefix={prefix}
          profileId={activeId ?? ""}
          onEnterPrefix={enterPrefix}
          onSelectBucket={selectBucket}
          onUp={goUp}
          onSelectObject={setSelected}
          onError={(e) => pushS3Error("操作失败", e)}
          onPresignSuccess={onPresignSuccess}
          onDeleteObject={(o) => setDeleteObjTarget(o)}
          checkedKeys={checkedKeys}
          onToggleCheck={toggleCheck}
          onToggleAll={toggleAllObjects}
          onDeleteMany={(keys) => setDeleteManyTarget([...keys])}
          onDownload={doDownload}
          onLoadMore={loadMoreObjects}
          onDeleteBucket={(b) => setBucketDeleteTarget(b)}
        />

        <TransferBar tasks={tasks} onClearTask={(taskId) => setTasks((ts) => ts.filter((t) => t.taskId !== taskId))} />

        <DetailPanel
          open={!!selected}
          object={selected}
          bucket={bucket ?? ""}
          profileId={activeId ?? ""}
          onClose={() => setSelected(null)}
          onError={(e) => pushS3Error("presign", e)}
        />

        {editTarget && (
          <AddClusterDialog
            editing={editTarget}
            onClose={() => setEditTarget(null)}
            onSaved={(p) => {
              setEditTarget(null);
              setProfiles((ps) => ps.map((x) => (x.id === p.id ? p : x)));
              if (p.id === activeId) loadBuckets(p.id);
            }}
          />
        )}
        {addOpen && (
          <AddClusterDialog
            onClose={() => setAddOpen(false)}
            onSaved={(p) => { setProfiles((ps) => [...ps, p]); setAddOpen(false); }}
          />
        )}
        </ErrorBoundary>
        {cmdkOpen && <Cmdk actions={cmdkActions} onClose={() => setCmdkOpen(false)} />}
        {deleteTarget && (
          <Dialog onClose={() => (deletingCluster ? null : setDeleteTarget(null))}>
            <h3 className="danger">删除集群？</h3>
            <p>将从本机移除集群 <strong>{deleteTarget.name}</strong>（endpoint <strong>{deleteTarget.endpoint}</strong>）。</p>
            <p>仅删除本地连接配置与本机保存的密钥，不会影响远端集群的任何数据。</p>
            <div className="foot">
              <button className="btn btn-secondary" onClick={() => setDeleteTarget(null)} disabled={deletingCluster}>取消</button>
              <button className="btn btn-danger" onClick={confirmDelete} disabled={deletingCluster}>
                {deletingCluster && <IconSpinner size={13} className="spin" />}
                {deletingCluster ? "删除中…" : "删除"}
              </button>
            </div>
          </Dialog>
        )}
        {deleteObjTarget && (
          <Dialog onClose={() => (deletingObj ? null : setDeleteObjTarget(null))}>
            <h3 className="danger">删除对象？</h3>
            <p>将从 <strong>{bucket}</strong> 永久删除对象 <strong>{deleteObjTarget.key}</strong>。</p>
            <p>此操作不可恢复；同名对象再次上传前将一直 404。</p>
            <div className="foot">
              <button className="btn btn-secondary" onClick={() => setDeleteObjTarget(null)} disabled={deletingObj}>取消</button>
              <button className="btn btn-danger" onClick={confirmDeleteObject} disabled={deletingObj}>
                {deletingObj && <IconSpinner size={13} className="spin" />}
                {deletingObj ? "删除中…" : "删除"}
              </button>
            </div>
          </Dialog>
        )}
        {deleteManyTarget && deleteManyTarget.length > 0 && (
          <Dialog onClose={() => (deletingMany ? null : setDeleteManyTarget(null))}>
            <h3 className="danger">删除 {deleteManyTarget.length} 项？</h3>
            <p>将从 <strong>{bucket}/{prefix}</strong> 永久删除以下内容：</p>
            <ul className="del-list mono">
              {deleteManyTarget.slice(0, 8).map((k) => (
                <li key={k} title={k}>{k.endsWith("/") ? "📁 " : ""}{safeDecode(k)}</li>
              ))}
              {deleteManyTarget.length > 8 && <li>… 及另外 {deleteManyTarget.length - 8} 项</li>}
            </ul>
            {deleteManyTarget.some((k) => k.endsWith("/")) && (
              <p>标记 <strong>📁</strong> 的是文件夹：其内部全部对象（含子文件夹）都会被递归删除。</p>
            )}
            <p>此操作不可恢复。</p>
            {deletingMany && batchProg && (
              <div className="del-progress">
                <div className="del-progress-text">
                  {batchProg.phase === "listing"
                    ? "正在展开文件夹中的对象…"
                    : `正在批量删除 ${batchProg.done} / ${batchProg.total}`}
                  <span className="del-progress-hint">（每批 1000 个对象，实时进度）</span>
                </div>
                <div
                  className="del-progress-bar"
                  role="progressbar"
                  aria-valuemin={0}
                  aria-valuemax={batchProg.total || 100}
                  aria-valuenow={batchProg.done}
                >
                  <div
                    className="del-progress-fill"
                    style={{ width: batchProg.total ? `${Math.min(100, (batchProg.done / batchProg.total) * 100)}%` : "30%" }}
                  />
                </div>
              </div>
            )}
            <div className="foot">
              <button className="btn btn-secondary" onClick={() => setDeleteManyTarget(null)} disabled={deletingMany}>取消</button>
              <button className="btn btn-danger" onClick={confirmDeleteObjects} disabled={deletingMany}>
                {deletingMany && <IconSpinner size={13} className="spin" />}
                {deletingMany ? "删除中…" : `删除 ${deleteManyTarget.length} 个`}
              </button>
            </div>
          </Dialog>
        )}
        {newBucketOpen && (
          <Dialog onClose={() => setNewBucketOpen(false)}>
            <h3>新建桶</h3>
            <p>将在集群 <strong>{active?.name}</strong> 上创建桶。桶名需小写字母/数字/连字符（3–63 字符）。</p>
            <input className="input mono" placeholder="my-new-bucket" value={newBucketName}
              onChange={(e) => setNewBucketName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && doCreateBucket()} />
            <div className="foot">
              <button className="btn btn-secondary" onClick={() => setNewBucketOpen(false)}>取消</button>
              <button className="btn btn-primary" disabled={!newBucketName.trim()} onClick={doCreateBucket}>创建</button>
            </div>
          </Dialog>
        )}
        {newFolderOpen && (
          <Dialog onClose={() => setNewFolderOpen(false)}>
            <h3>新建文件夹</h3>
            <p>在 <strong className="mono">{bucket}{prefix && `/${prefix.replace(/\/$/, "")}`}</strong> 下创建文件夹。S3 中文件夹是 key 以 <span className="mono">/</span> 结尾的 0 字节占位对象。</p>
            <input className="input mono" placeholder="folder-name" value={newFolderName}
              onChange={(e) => setNewFolderName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && doCreateFolder()} />
            <div className="foot">
              <button className="btn btn-secondary" onClick={() => setNewFolderOpen(false)}>取消</button>
              <button className="btn btn-primary" disabled={!newFolderName.trim()} onClick={doCreateFolder}>创建</button>
            </div>
          </Dialog>
        )}
        {bucketDeleteTarget && (
          <Dialog onClose={() => (deletingBucket ? null : setBucketDeleteTarget(null))}>
            <h3 className="danger">删除桶？</h3>
            <p>将删除集群 <strong>{active?.name}</strong> 上的桶 <strong>{bucketDeleteTarget}</strong>。</p>
            <p>仅空桶可删除；桶内任何对象都会导致 <span className="mono">BucketNotEmpty</span> 失败。</p>
            <div className="foot">
              <button className="btn btn-secondary" onClick={() => setBucketDeleteTarget(null)} disabled={deletingBucket}>取消</button>
              <button className="btn btn-danger" onClick={doDeleteBucket} disabled={deletingBucket}>
                {deletingBucket && <IconSpinner size={13} className="spin" />}
                {deletingBucket ? "删除中…" : "删除"}
              </button>
            </div>
          </Dialog>
        )}
        <Toasts toasts={toasts} onDismiss={dismiss} />
      </div>
    </div>
  );
}
