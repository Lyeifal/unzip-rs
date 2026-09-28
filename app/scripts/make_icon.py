"""生成 app 图标源图：1024x1024 蓝底圆角方块 + 白色「解」字。"""
from PIL import Image, ImageDraw, ImageFont

SIZE = 1024
RADIUS = 180
BG = (47, 129, 247, 255)  # #2f81f7 主按钮蓝

img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)
draw.rounded_rectangle([0, 0, SIZE - 1, SIZE - 1], radius=RADIUS, fill=BG)

font = ImageFont.truetype(r"C:\Windows\Fonts\msyhbd.ttc", 560)
text = "解"
bbox = draw.textbbox((0, 0), text, font=font)
tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
pos = ((SIZE - tw) / 2 - bbox[0], (SIZE - th) / 2 - bbox[1])
draw.text(pos, text, font=font, fill=(255, 255, 255, 255))

img.save(r"D:\Code\unzip-rs\app\src-tauri\icons\icon.png")
print("icon.png written", img.size)
