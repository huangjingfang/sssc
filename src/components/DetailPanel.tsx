import { useEffect, useRef, useState } from "react";
import { formatSize } from "../lib/backend";
import { api } from "../lib/backend";
import type { S3Object } from "../types";
import { IconClose, IconSpinner } from "./icons";
import { copyText } from "./overlays";

export default function DetailPanel(props: {
  open: boolean;
  object: S3Object | null;
  bucket: string;
  profileId: string;
  onClose: () => void;
  onError: (e: unknown) => void;
}) {
  const [expires, setExpires] = useState(86400);
  const [url, setUrl] = useState<string | null>(null);
  const [signing, setSigning] = useState(false);
  const genSeq = useRef(0);

  // 桶或 key 变化（切换选中对象）时重置旧 URL
  useEffect(() => { setUrl(null); }, [props.object?.key, props.bucket]);

  const gen = async () => {
    if (!props.object) return;
    const seq = ++genSeq.current;
    setSigning(true);
    try {
      const u = await api.presignGet(props.profileId, props.bucket, props.object.key, expires);
      // 序号守卫：连续生成时丢弃过期响应，防止旧结果覆盖新结果
      if (genSeq.current !== seq) return;
      setUrl(u);
    } catch (e) { props.onError(e); } finally {
      if (genSeq.current === seq) setSigning(false);
    }
  };

  const o = props.object;
  return (
    <div className={`detail ${props.open && o ? "open" : ""}`}>
      {o && (
        <>
          <div className="detail-head">
            <span className="t mono">{o.key.split("/").pop()}</span>
            <button className="icon-btn" onClick={props.onClose}><IconClose /></button>
          </div>
          <div className="detail-body">
            <div className="section-t">元数据 (HeadObject)</div>
            <div className="kv">
              <span className="k">Key</span><span className="v">{o.key}</span>
              <span className="k">大小</span><span className="v">{formatSize(o.size)}{o.size != null ? ` (${o.size.toLocaleString()} bytes)` : ""}</span>
              <span className="k">ETag</span><span className="v">{o.etag ?? "—"}</span>
              <span className="k">存储类</span><span className="v">{o.storageClass ?? "STANDARD"}</span>
              <span className="k">LastModified</span><span className="v">{o.lastModified ?? "—"}</span>
            </div>

            <div className="section-t">预签名链接</div>
            <div className="presign-card">
              <div className="row">
                <select className="select" style={{ flex: 1 }} value={expires} onChange={(e) => setExpires(Number(e.target.value))}>
                  <option value={3600}>1 小时</option>
                  <option value={86400}>24 小时</option>
                  <option value={604800}>7 天（上限）</option>
                </select>
                <button className="btn btn-secondary" onClick={gen} disabled={signing}>
                  {signing && <IconSpinner size={13} className="spin" />} 生成
                </button>
              </div>
              {url && (
                <>
                  <div className="url-box">{url}</div>
                  <div className="row" style={{ marginTop: 10, marginBottom: 0 }}>
                    <button className="btn btn-primary" style={{ flex: 1 }}
                      onClick={async () => { if (await copyText(url)) setTimeout(() => props.onClose(), 200); }}>
                      复制链接
                    </button>
                  </div>
                </>
              )}
            </div>
            <p style={{ fontSize: 12, color: "var(--ds-fg-3)" }}>
              SigV4 预签名最长 7 天；使用 STS 临时凭证签名时，链接随凭证过期提前失效。
            </p>
          </div>
        </>
      )}
    </div>
  );
}
