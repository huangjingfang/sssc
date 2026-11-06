import { useState } from "react";
import type { KeyboardEvent } from "react";
import { formatSize, formatTime, safeDecode } from "../lib/backend";
import type { Bucket, ListResult, S3Object } from "../types";
import { IconDownload, IconFile, IconFolder, IconLink, IconSpinner, IconTrash } from "./icons";
import { copyText, presignAndCopy } from "./overlays";

const activateOnKey = (fn: () => void) => (e: KeyboardEvent) => {
  if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); fn(); }
};

export default function ObjectTable(props: {
  loading: boolean;
  /** 追加页加载中（“加载更多”进行中） */
  loadingMore: boolean;
  list: ListResult | null;
  buckets: Bucket[] | null;
  bucket: string | null;
  prefix: string;
  profileId: string;
  onEnterPrefix: (p: string) => void;
  onSelectBucket: (b: string) => void;
  onUp: () => void;
  onSelectObject: (o: S3Object) => void;
  onError: (e: unknown) => void;
  /** 预签名复制成功（含有效期秒数），由上层给 toast */
  onPresignSuccess: (key: string, expiresSec: number) => void;
  /** 请求删除对象（上层弹确认对话框） */
  onDeleteObject: (o: S3Object) => void;
  /** 多选：当前勾选的对象 key 集合 */
  checkedKeys: string[];
  /** 多选：切换某个对象的勾选状态 */
  onToggleCheck: (key: string) => void;
  /** 多选：全选/清空当前页所有对象 */
  onToggleAll: () => void;
  /** 多选：请求批量删除（上层弹确认对话框） */
  onDeleteMany: (keys: string[]) => void;
  onDownload: (key: string) => void;
  onLoadMore: () => void;
  onDeleteBucket: (bucket: string) => void;
}) {
  const [busyKey, setBusyKey] = useState<string | null>(null);

  const doPresign = async (key: string) => {
    setBusyKey(key);
    try {
      const ok = await presignAndCopy(props.profileId, props.bucket!, key, 86400);
      if (ok) props.onPresignSuccess(key, 86400);
    } catch (e) { props.onError(e); } finally { setBusyKey(null); }
  };

  /* 顶部细条加载指示：不整表替换，保留旧数据与滚动位置 */
  const loadingBar = props.loading && (
    <div className="list-loading-bar" role="progressbar" aria-label="加载中" aria-busy="true" />
  );

  /* 桶列表模式 */
  if (!props.bucket) {
    // 首次尚无数据才显示骨架；已有数据时叠加 loading 指示
    if (props.buckets == null) {
      return (
        <>
          {loadingBar}
          <div className="loading-row"><IconSpinner className="spin" /> 加载中…</div>
        </>
      );
    }
    return (
      <>
        {loadingBar}
        <div className="table-wrap">
          <table>
            <thead><tr><th>桶名称</th><th>区域</th><th>创建时间</th><th style={{ width: 110 }} /></tr></thead>
            <tbody>
              {props.buckets.map((b) => (
                <tr key={b.name}>
                  <td>
                    <div
                      className="name link"
                      role="button"
                      tabIndex={0}
                      title={safeDecode(b.name)}
                      onClick={() => props.onSelectBucket(b.name)}
                      onKeyDown={activateOnKey(() => props.onSelectBucket(b.name))}
                    >
                      <IconFolder /><span className="mono">{safeDecode(b.name)}</span>
                    </div>
                  </td>
                  <td className="mono" style={{ fontSize: 12 }}>{b.region ?? "—"}</td>
                  <td className="time">{formatTime(b.createdAt)}</td>
                  <td>
                    <div className="row-actions">
                      <button className="icon-btn sm" title="删除桶（仅空桶）" onClick={() => props.onDeleteBucket(b.name)}>
                        <IconTrash size={14} />
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </>
    );
  }

  /* 对象列表模式（ListObjectsV2 + delimiter=/） */
  const list = props.list;
  if (list == null) {
    return (
      <>
        {loadingBar}
        <div className="loading-row"><IconSpinner className="spin" /> 加载中…</div>
      </>
    );
  }
  const checked = new Set(props.checkedKeys);
  const objectKeys = [...list.prefixes, ...list.objects.map((o) => o.key)];
  const allChecked = objectKeys.length > 0 && objectKeys.every((k) => checked.has(k));
  const folderCount = props.checkedKeys.filter((k) => k.endsWith("/")).length;
  const toggleAll = () => props.onToggleAll();
  return (
    <>
      {loadingBar}
      {props.checkedKeys.length > 0 && (
        <div className="batch-bar" role="toolbar" aria-label="批量操作">
          <span>
            已选 <strong>{props.checkedKeys.length}</strong> 项
            {folderCount > 0 && <>（含文件夹 <strong>{folderCount}</strong> 个，将递归删除其中全部内容）</>}
          </span>
          <div className="spacer" />
          <button className="btn btn-secondary" onClick={() => props.onToggleAll()}>
            {allChecked ? "取消全选" : "全选"}
          </button>
          <button className="btn btn-danger" onClick={() => props.onDeleteMany(props.checkedKeys)}>
            <IconTrash size={14} /> 删除所选
          </button>
          <button className="btn btn-secondary" onClick={() => objectKeys.forEach((k) => { if (checked.has(k)) props.onToggleCheck(k); })}>
            取消选择
          </button>
        </div>
      )}
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th className="sel-col">
                <input
                  type="checkbox"
                  aria-label={allChecked ? "取消全选" : "全选本页对象"}
                  checked={allChecked}
                  onChange={toggleAll}
                />
              </th>
              <th style={{ width: "46%" }}>名称</th>
              <th className="num">大小</th>
              <th>存储类</th>
              <th>最后修改</th>
              <th style={{ width: 110 }} />
            </tr>
          </thead>
          <tbody>
            {props.prefix && (
              <tr>
                <td className="sel-col" />
                <td>
                  <div className="name link" role="button" tabIndex={0} onClick={props.onUp} onKeyDown={activateOnKey(props.onUp)}>
                    <IconFolder /><span className="mono">../</span>
                  </div>
                </td>
                <td className="num" /><td /><td /><td />
              </tr>
            )}
            {list.prefixes.map((p) => (
              <tr key={p}>
                <td className="sel-col" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                  <input
                    type="checkbox"
                    aria-label={`选择文件夹 ${safeDecode(p)}`}
                    checked={checked.has(p)}
                    onChange={() => props.onToggleCheck(p)}
                  />
                </td>
                <td>
                  <div
                    className="name link"
                    role="button"
                    tabIndex={0}
                    title={safeDecode(p)}
                    onClick={() => props.onEnterPrefix(p)}
                    onKeyDown={activateOnKey(() => props.onEnterPrefix(p))}
                  >
                    <IconFolder /><span className="mono">{safeDecode(p.slice(props.prefix.length))}</span>
                  </div>
                </td>
                <td className="sel-col" /><td /><td className="num">—</td><td /><td className="time" /><td />
              </tr>
            ))}
            {list.objects.map((o) => (              <tr
                key={o.key}
                tabIndex={0}
                className={checked.has(o.key) ? "row-checked" : undefined}
                onClick={() => props.onSelectObject(o)}
                onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); props.onSelectObject(o); } }}
              >
                <td className="sel-col" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                  <input
                    type="checkbox"
                    aria-label={`选择 ${safeDecode(o.key)}`}
                    checked={checked.has(o.key)}
                    onChange={() => props.onToggleCheck(o.key)}
                  />
                </td>
                <td>
                  <div className="name">
                    <IconFile />
                    <span className="mono" title={safeDecode(o.key)}>{safeDecode(o.key.slice(props.prefix.length))}</span>
                  </div>
                </td>
                <td className="num mono">{formatSize(o.size)}</td>
                <td><span className={`chip ${o.storageClass === "GLACIER" || o.storageClass === "DEEP_ARCHIVE" ? "warning" : "neutral"}`}>
                  {o.storageClass ?? "STANDARD"}
                </span></td>
                <td className="time">{formatTime(o.lastModified)}</td>
                <td>
                  <div className="row-actions" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
                    <button className="icon-btn sm" title="生成预签名链接（24h）并复制" disabled={busyKey === o.key}
                      onClick={() => doPresign(o.key)}>
                      {busyKey === o.key ? <IconSpinner size={13} className="spin" /> : <IconLink size={14} />}
                    </button>
                    <button className="icon-btn sm" title="下载到本地…" onClick={() => props.onDownload(o.key)}>
                      <IconDownload size={14} />
                    </button>
                    <button className="icon-btn sm" title="删除" onClick={() => props.onDeleteObject(o)}>
                      <IconTrash size={14} />
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {list.nextToken && (
          <div className="load-more">
            <button className="btn btn-secondary" disabled={props.loadingMore} onClick={props.onLoadMore}>
              {props.loadingMore && <IconSpinner size={13} className="spin" />}
              {props.loadingMore ? "加载中…" : "加载更多"}
            </button>
          </div>
        )}
      </div>
    </>
  );
}

export function useObjectActions() {
  // 预留给传输引擎的 hook 占位
  return { copy: copyText };
}
