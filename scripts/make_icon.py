"""生成 SSSC 应用图标（1024x1024）：明亮渐变圆角方块 + 白色云朵上传。

与前端 IconLogo 同款设计：白色填充云朵，云内镂空上传箭头。
"""
from PIL import Image, ImageDraw
import numpy as np

S = 2048   # 超采样画布
FIN = 1024  # 最终输出
U = 28.0    # 与前端 SVG 相同的 viewBox 坐标系


def u(v):
    return (v + 1.25) * S / U


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


C1, C2, C3 = (0x38, 0xBD, 0xF8), (0x60, 0xA5, 0xFA), (0xFB, 0xBF, 0x24)

# 1) 对角渐变背景
yy, xx = np.mgrid[0:S, 0:S]
t = (xx + yy) / (2 * S)
t2 = np.clip(t / 0.55, 0, 1)
t3 = np.clip((t - 0.55) / 0.45, 0, 1)
rgb = np.stack([
    np.where(t < 0.55, C1[0] + (C2[0] - C1[0]) * t2, C2[0] + (C3[0] - C2[0]) * t3),
    np.where(t < 0.55, C1[1] + (C2[1] - C1[1]) * t2, C2[1] + (C3[1] - C2[1]) * t3),
    np.where(t < 0.55, C1[2] + (C2[2] - C1[2]) * t2, C2[2] + (C3[2] - C2[2]) * t3),
], axis=-1).astype(np.uint8)

# 圆角矩形蒙版（rx = 8/28）
mask_img = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask_img).rounded_rectangle([0, 0, S - 1, S - 1], radius=u(8), fill=255)
rounded = np.array(mask_img) > 0

alpha = np.zeros((S, S), dtype=np.uint8)
alpha[rounded] = 255
img = Image.fromarray(np.dstack([rgb, alpha]), "RGBA")

# 2) 白色填充云朵（三圆 + 底部圆角矩形），坐标在 28 viewBox 系
cloud = Image.new("L", (S, S), 0)
cd = ImageDraw.Draw(cloud)
cd.ellipse([u(5.5), u(10.3), u(11.7), u(16.5)], fill=255)    # 左圆
cd.ellipse([u(8.8), u(7.3), u(17.4), u(15.9)], fill=255)     # 中大圆
cd.ellipse([u(14.5), u(10.6), u(20.0), u(16.1)], fill=255)   # 右圆
cd.rounded_rectangle([u(5.5), u(13.0), u(20.0), u(17.3)], radius=u(2.0), fill=255)

# 3) 上传箭头（粗线 + 圆头），中心 x=12.75
arrow = Image.new("L", (S, S), 0)
ad = ImageDraw.Draw(arrow)
w = u(1.9)
cx = u(12.75)


def line(p1, p2):
    ad.line([p1, p2], fill=255, width=int(w))
    r = w / 2
    for p in (p1, p2):
        ad.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=255)


tip = (cx, u(12.2))
line(tip, (cx, u(21.0)))                       # 竖线
line(tip, (u(9.55), u(14.85)))                 # 左斜
line(tip, (u(15.95), u(14.85)))                # 右斜

cloud_a = np.array(cloud)
arrow_a = np.array(arrow)
inside = arrow_a & cloud_a          # 云内部分 → 镂空
outside = arrow_a & ~cloud_a        # 云下部分 → 白色

white = ((cloud_a & ~inside) | outside) > 0

arr = np.array(img)
arr[..., :3][white] = 255
arr[..., 3][white] = 255
out = Image.fromarray(arr, "RGBA").resize((FIN, FIN), Image.LANCZOS)
out.save("src-tauri/icons/icon-source.png")
print("saved src-tauri/icons/icon-source.png")
