import { useState } from "react";
import { api } from "../lib/backend";
import type { ConnectionStatus, Profile } from "../types";
import { Dialog } from "./overlays";
import { IconSpinner, IconCheck, IconWarn } from "./icons";

const COLORS = ["red", "green", "blue", "purple", "amber"];

const emptyForm = {
  name: "",
  endpoint: "",
  region: "us-east-1",
  accessKey: "",
  secretKey: "",
  style: "path" as Profile["style"],
  signature: "v4" as Profile["signature"],
  insecureTls: false,
  color: "blue",
};

export default function AddClusterDialog(props: {
  onClose: () => void;
  onSaved: (p: Profile) => void;
  /** 编辑模式：预填已有 Profile；Secret 留空 = 保持原密钥 */
  editing?: Profile | null;
}) {
  const e = props.editing ?? null;
  const [form, setForm] = useState(
    e
      ? {
          name: e.name,
          endpoint: e.endpoint,
          region: e.region,
          accessKey: e.accessKey,
          secretKey: "",
          style: e.style,
          signature: (e.signature ?? "v4") as Profile["signature"],
          insecureTls: e.insecureTls,
          color: e.color,
        }
      : emptyForm,
  );
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<ConnectionStatus | null>(null);
  const editing = e !== null;
  /** 编辑模式下该集群后端是否已存密钥（secretMasked 为空 = 从未保存过） */
  const needSecret = editing && !e?.secretMasked;

  const draft: Profile = {
    id: e?.id ?? `p_${Date.now()}`,
    name: form.name,
    endpoint: form.endpoint,
    region: form.region,
    accessKey: form.accessKey,
    // 仅在填写时回传；留空 = 保持后端已存密钥（若有）
    secretKey: form.secretKey || undefined,
    secretMasked: form.secretKey
      ? `••••${form.secretKey.slice(-4)}`
      : e?.secretMasked ?? "",
    style: form.style,
    signature: form.signature,
    insecureTls: form.insecureTls,
    color: form.color,
    enabled: true,
  };

  const valid =
    form.name && form.endpoint && form.accessKey && (editing ? !needSecret || !!form.secretKey : !!form.secretKey);

  const test = async () => {
    setTesting(true);
    setStatus(null);
    try {
      // 编辑模式未重填密钥：secretKey 留空（undefined）回传原 profile 的 id 等字段，
      // 后端 test_connection 对已存密钥的 profile 使用本机保存的真实密钥完成测试。
      const profileForTest: Profile = editing && !form.secretKey && e
        ? { ...draft, secretKey: undefined }
        : draft;
      setStatus(await api.testConnection(profileForTest));
    } catch (err) {
      setStatus({ ok: false, latencyMs: null, bucketCount: null, error: err as never });
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    if (saving) return;
    setSaving(true);
    try {
      // Secret 留空（编辑模式）= 保持原密钥，不回传 secretKey
      props.onSaved(await api.saveProfile(draft));
    } catch (err) {
      setStatus({ ok: false, latencyMs: null, bucketCount: null, error: err as never });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog onClose={props.onClose}>
      <h3>{editing ? "编辑集群" : "连接你的第一个集群"}</h3>
      <p>Profile = endpoint + region + 凭证 + 寻址风格。密钥仅存本机（secrets.json，0600）。</p>

      <div className="form-row">
        <div className="form-field">
          <label>显示名</label>
          <input className="input" placeholder="prod-minio" value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })} />
        </div>
        <div className="form-field" style={{ flex: "0 0 150px" }}>
          <label>标识色</label>
          <div className="color-picker" role="radiogroup" aria-label="标识色">
            {COLORS.map((c) => (
              <button
                key={c}
                type="button"
                role="radio"
                aria-checked={form.color === c}
                title={c}
                className={`color-dot ${form.color === c ? "sel" : ""}`}
                onClick={() => setForm({ ...form, color: c })}
              >
                <span className="swatch" style={{ background: `var(--cluster-${c})` }} />
              </button>
            ))}
          </div>
        </div>
      </div>
      <div className="form-field" style={{ marginBottom: 10 }}>
        <label>Endpoint</label>
        <input className="input mono" placeholder="https://s3.example.com:9000" value={form.endpoint}
          onChange={(e) => setForm({ ...form, endpoint: e.target.value })} />
      </div>
      <div className="form-row">
        <div className="form-field">
          <label>Access Key</label>
          <input className="input mono" value={form.accessKey}
            onChange={(e) => setForm({ ...form, accessKey: e.target.value })} />
        </div>
        <div className="form-field">
          <label>
            Secret Key{" "}
            {editing
              ? needSecret
                ? "（必填：该集群尚未保存过密钥）"
                : "（留空 = 保持不变）"
              : ""}
          </label>
          <input className="input mono" type="password" placeholder={editing ? e?.secretMasked || "保持原密钥" : ""}
            value={form.secretKey}
            onChange={(ev) => setForm({ ...form, secretKey: ev.target.value })} />
        </div>
      </div>
      <div className="form-row">
        <div className="form-field">
          <label>Region（留空 = us-east-1）</label>
          <input className="input mono" placeholder="us-east-1" value={form.region}
            onChange={(e) => setForm({ ...form, region: e.target.value })} />
        </div>
        <div className="form-field">
          <label>签名版本</label>
          <select className="select" value={form.signature} onChange={(e) => setForm({ ...form, signature: e.target.value as Profile["signature"] })}>
            <option value="v4">SigV4（默认）</option>
            <option value="v2">SigV2（nginx 反代 RGW 等旧网关）</option>
          </select>
        </div>
      </div>
      <div className="form-row">
        <div className="form-field">
          <label>寻址风格</label>
          <select className="select" value={form.style} onChange={(e) => setForm({ ...form, style: e.target.value as Profile["style"] })}>
            <option value="path">path-style（私有部署推荐）</option>
            <option value="virtual">virtual-hosted</option>
          </select>
        </div>
      </div>
      <div className="check-row">
        <input type="checkbox" id="insecure" checked={form.insecureTls}
          onChange={(e) => setForm({ ...form, insecureTls: e.target.checked })} />
        <label htmlFor="insecure">信任自签证书（仅研发环境）</label>
      </div>

      {status && (
        <p style={{ display: "flex", alignItems: "center", gap: 6 }}>
          {status.ok
            ? <><IconCheck size={14} /> 连接成功 · {status.latencyMs ?? "—"}ms · {status.bucketCount ?? "—"} 桶</>
            : <><IconWarn size={14} /> {status.error?.code}: {status.error?.message}</>
          }
        </p>
      )}

      <div className="foot">
        <button
          className="btn btn-secondary"
          disabled={!valid || testing}
          onClick={test}
          title={editing && !form.secretKey ? "使用本机已保存的密钥测试连接" : ""}
        >
          {testing && <IconSpinner size={13} className="spin" />} 测试连接
        </button>
        <button className="btn btn-primary" disabled={!valid || saving} onClick={save}>
          {saving && <IconSpinner size={13} className="spin" />} 保存
        </button>
      </div>
    </Dialog>
  );
}
