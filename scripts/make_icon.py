"""生成 SSSC 应用图标（1024x1024）。

与前端 IconLogo 同款设计语言：明亮渐变圆角方块 + 白色云朵（描边轮廓）
+ 上传箭头。云朵用「外形状 − 内缩形状」得到均匀描边，几何比例对齐前端。
"""
from PIL import Image, ImageDraw
import numpy as np

S = 2048
FIN = 1024
U = 28.0
SW = 1.7  # 描边宽度（viewBox 单位），与前端一致

# 内容包围盒：x 4.8~20.6, y 7.6~22.0 → 居中平移
BX0, BY0, BX1, BY1 = 4.8, 7.6, 20.6, 22.0
TX = U / 2 - (BX0 + BX1) / 2
TY = U / 2 - (BY0 + BY1) / 2


def u(v):
    return (v + TX) * S / U


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


C1, C2, C3 = (0x38, 0xBD, 0xF8), (0x60, 0xA5, 0xFA), (0xFB, 0xBF, 0x24)

# 1) 对角渐变背景 + 圆角矩形（rx=8）
yy, xx = np.mgrid[0:S, 0:S]
t = (xx + yy) / (2 * S)
t2 = np.clip(t / 0.55, 0, 1)
t3 = np.clip((t - 0.55) / 0.45, 0, 1)
rgb = np.stack([
    np.where(t < 0.55, C1[0] + (C2[0] - C1[0]) * t2, C2[0] + (C3[0] - C2[0]) * t3),
    np.where(t < 0.55, C1[1] + (C2[1] - C1[1]) * t2, C2[1] + (C3[1] - C2[1]) * t3),
    np.where(t < 0.55, C1[2] + (C2[2] - C1[2]) * t2, C2[2] + (C3[2] - C2[2]) * t3),
], axis=-1).astype(np.uint8)
mask_img = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask_img).rounded_rectangle([0, 0, S - 1, S - 1], radius=8 * S / U, fill=255)
rounded = np.array(mask_img) > 0
alpha = np.zeros((S, S), dtype=np.uint8)
alpha[rounded] = 255
img = Image.fromarray(np.dstack([rgb, alpha]), "RGBA")

# 2) 云朵描边轮廓：填充云 − 腐蚀云（高斯模糊阈值近似圆盘腐蚀，干净无毛刺）
from PIL import ImageFilter

# 外云：三个圆 + 底部圆角矩形（viewBox 坐标，底边对齐 y=17.5，整体加大）
CLOUD_CIRCLES = [
    (9.0, 13.9, 4.2),   # 左
    (13.8, 12.6, 5.0),  # 中（顶）
    (17.0, 14.3, 3.6),  # 右
]
CLOUD_RECT = (5.6, 13.3, 19.2, 17.5, 2.2)

filled_img = Image.new("L", (S, S), 0)
fd = ImageDraw.Draw(filled_img)
for cx, cy, r in CLOUD_CIRCLES:
    fd.ellipse([u(cx - r), u(cy - r), u(cx + r), u(cy + r)], fill=255)
x0, y0, x1, y1, rr = CLOUD_RECT
fd.rounded_rectangle([u(x0), u(y0), u(x1), u(y1)], radius=u(rr), fill=255)

filled = np.array(filled_img) > 0
wpx = 2.2 * S / U          # 云朵描边加粗
arrow_wpx = 1.35 * S / U   # 箭头描边调细
sigma = wpx / 4  # σ ≈ 半描边宽的一半，保证腐蚀深度 ≈ wpx/2
eroded = np.array(filled_img.filter(ImageFilter.GaussianBlur(radius=sigma))) >= int(0.977 * 255)
eroded &= filled  # 防止模糊外溢
outline = filled & ~eroded

# 3) 上传箭头（描边、圆头）：M14 22 V15.5 + 两翼
arrow = Image.new("L", (S, S), 0)
ad = ImageDraw.Draw(arrow)
arrow_pad = Image.new("L", (S, S), 0)  # 箭头外扩一圈（含间隙），用于在云朵描边上开断口
apd = ImageDraw.Draw(arrow_pad)
gap = 0.55 * S / U  # 云朵与箭头之间的渐变色缝隙宽度


def stroke(p1, p2):
    a, b = (u(p1[0]), u(p1[1])), (u(p2[0]), u(p2[1]))
    ad.line([a, b], fill=255, width=int(arrow_wpx))
    r = arrow_wpx / 2
    for p in (a, b):
        ad.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=255)
    rp = (arrow_wpx + 2 * gap) / 2
    apd.line([a, b], fill=255, width=int(arrow_wpx + 2 * gap))
    for p in (a, b):
        apd.ellipse([p[0] - rp, p[1] - rp, p[0] + rp, p[1] + rp], fill=255)


stroke((14, 21.6), (14, 15.8))
stroke((11.6, 18.2), (14, 15.8))
stroke((16.4, 18.2), (14, 15.8))

# 云朵描边在箭头经过处断开，形成边界缝隙
outline = outline & ~(np.array(arrow_pad) > 0)

# 4) 合成：云朵轮廓 + 箭头 全部白色
wa = outline | (np.array(arrow) > 0)
arr = np.array(img)
arr[..., :3][wa] = 255
arr[..., 3][wa] = 255
out = Image.fromarray(arr, "RGBA").resize((FIN, FIN), Image.LANCZOS)
out.save("src-tauri/icons/icon-source.png")
print("saved")
