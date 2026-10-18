"""Generate a black rounded archive icon on a transparent canvas."""
from pathlib import Path
from PIL import Image, ImageDraw

SIZE = 1024
APP = Path(__file__).resolve().parents[1]
INK = (0, 0, 0, 255)

img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)
draw.rounded_rectangle((128, 96, 896, 928), radius=144, outline=INK, width=72)

for index, top in enumerate(range(176, 576, 80)):
    left = 432 if index % 2 == 0 else 512
    draw.rounded_rectangle((left, top, left + 80, top + 56), radius=12, fill=INK)
draw.rounded_rectangle((416, 592, 608, 808), radius=64, fill=INK)
draw.rounded_rectangle((472, 672, 552, 744), radius=22, fill=(0, 0, 0, 0))

img.save(APP / "src-tauri/icons/icon.png")
img.resize((256, 256), Image.Resampling.LANCZOS).save(APP / "public/app-icon.png")
print("Generated black transparent archive icon")
