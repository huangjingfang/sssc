/** 线性图标（1.5px stroke，Lucide 风格），见设计规范 §6 */
type P = { size?: number; className?: string };
const S = (d: React.ReactNode, { size = 16, className }: P = {}) => (
  <svg className={className} width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">{d}</svg>
);

export const IconMenu = (p: P) => S(<><path d="M3 6h18M3 12h18M3 18h18" /></>, p);
export const IconSearch = (p: P) => S(<><circle cx="11" cy="11" r="7" /><path d="m21 21-4-4" /></>, p);
export const IconRefresh = (p: P) => S(<><path d="M21 12a9 9 0 1 1-2.6-6.3L21 8" /><path d="M21 3v5h-5" /></>, p);
export const IconUpload = (p: P) => S(<><path d="M12 15V3m0 0L8 7m4-4 4 4M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" /></>, p);
export const IconDownload = (p: P) => S(<><path d="M12 3v12m0 0 4-4m-4 4-4-4M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" /></>, p);
export const IconFolder = (p: P) => S(<><path d="M4 20h16a1 1 0 0 0 1-1V8a1 1 0 0 0-1-1h-8l-2-3H4a1 1 0 0 0-1 1v14a1 1 0 0 0 1 1Z" /></>, p);
export const IconFile = (p: P) => S(<><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z" /><path d="M14 2v6h6" /></>, p);
export const IconTrash = (p: P) => S(<><path d="M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2m3 0-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" /></>, p);
export const IconLink = (p: P) => S(<><path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" /><path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7" /></>, p);
export const IconClose = (p: P) => S(<><path d="M18 6 6 18M6 6l12 12" /></>, p);
export const IconSun = (p: P) => S(<><circle cx="12" cy="12" r="4" /><path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" /></>, p);
export const IconMoon = (p: P) => S(<><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z" /></>, p);
export const IconWarn = (p: P) => S(<><path d="M12 8v4m0 4h.01M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z" /></>, p);
export const IconCheck = (p: P) => S(<><path d="M20 6 9 17l-5-5" /></>, p);
export const IconPlus = (p: P) => S(<><path d="M12 5v14M5 12h14" /></>, p);
export const IconEdit = (p: P) => S(<><path d="M17 3a2.8 2.8 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" /></>, p);
export const IconSpinner = (p: P) => S(<><path d="M21 12a9 9 0 1 1-6.2-8.6" /></>, p);
export const IconUnlink = (p: P) => S(<><path d="M17 7 8.5 15.5a3.5 3.5 0 0 0 5 5l3-3a3.5 3.5 0 0 0 0-5M8.5 15.5 7 17l-2 2m7.5-16.5 2-2a3.5 3.5 0 0 1 5 5l-2 2M3 8l2-2m11 13 2 2" /></>, p);

/** 品牌 Logo：明亮渐变（天蓝→阳光橙）圆角方块 + 白色云朵上传，简洁现代 */
export const IconLogo = ({ size = 28, className }: P) => (
  <svg className={className} width={size} height={size} viewBox="0 0 28 28" fill="none" aria-label="SSSC Logo">
    <defs>
      <linearGradient id="sssc-logo-g" x1="0" y1="0" x2="28" y2="28" gradientUnits="userSpaceOnUse">
        <stop offset="0" stopColor="#38BDF8" />
        <stop offset=".55" stopColor="#60A5FA" />
        <stop offset="1" stopColor="#FBBF24" />
      </linearGradient>
    </defs>
    <rect width="28" height="28" rx="8" fill="url(#sssc-logo-g)" />
    <path
      d="M9.2 17.5a3.2 3.2 0 0 1-.4-6.38 4.3 4.3 0 0 1 8.35-1.02 3.6 3.6 0 0 1 1.35 7.05"
      stroke="#fff" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"
    />
    <path d="M14 22v-6.5m0 0-2.6 2.6M14 15.5l2.6 2.6" stroke="#fff" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);
