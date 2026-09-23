#!/usr/bin/env python3
"""Generate synthetic OCR fixtures, never screen captures. Requires Pillow."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

root = Path(__file__).resolve().parent.parent / 'tests' / 'fixtures'
root.mkdir(parents=True, exist_ok=True)
font_path = '/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf'
examples = [
    ('dialog', 'The door is locked. Find the key.', 22, False, False),
    ('dark', 'Welcome to the village!', 20, True, False),
    ('small', 'Take the sword and shield.', 12, False, False),
    ('pixelated', 'The dragon is sleeping.', 11, True, True),
    ('multiline', 'Hello, traveler!\nYour quest begins here.', 20, False, False),
]
for name, text, size, dark, pixelated in examples:
    image = Image.new('L', (520 if not pixelated else 200, 90 if not pixelated else 38), 0 if dark else 255)
    draw = ImageDraw.Draw(image)
    draw.multiline_text((10, 10), text, font=ImageFont.truetype(font_path, size), fill=255 if dark else 0, spacing=8)
    if pixelated:
        image = image.point(lambda p: 255 if p >= 128 else 0).resize((600, 114), Image.Resampling.NEAREST)
    image.save(root / f'{name}.png')
    (root / f'{name}.txt').write_text(' '.join(text.split()) + '\n')
Image.new('L', (520, 90), 255).save(root / 'blank.png')
(root / 'blank.txt').write_text('\n')
