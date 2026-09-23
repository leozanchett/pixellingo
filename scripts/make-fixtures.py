#!/usr/bin/env python3
"""Generate synthetic OCR fixtures, never screen captures. Requires Pillow."""
import random
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

# A large game-like selection with a decorative background and a centered dialog.
image = Image.new('L', (640, 480), 25)
draw = ImageDraw.Draw(image)
for x in range(0, 640, 23):
    draw.line((x, 0, x + 40, 480), fill=32, width=3)
draw.rectangle((75, 210, 565, 325), fill=5, outline=110, width=3)
text = 'There is no saved game.\nWould you like to create a new file?'
draw.multiline_text((90, 235), text, font=ImageFont.truetype(font_path, 24), fill=245, spacing=10)
image.save(root / 'scenery.png')
(root / 'scenery.txt').write_text(' '.join(text.split()) + '\n')

# A short line at the top of a large moving scene. No game screenshots/assets.
for name, seed, text in [('sparse-dialog', 7, 'Are you all right?'),
                         ('sparse-dialog-moved', 19, 'Are you all right?'),
                         ('sparse-empty', 7, '')]:
    rng = random.Random(seed)
    image = Image.new('L', (640, 480), 80)
    draw = ImageDraw.Draw(image)
    for _ in range(1500):
        x, y = rng.randrange(640), rng.randrange(480)
        draw.ellipse((x, y, x + rng.randrange(2, 12), y + rng.randrange(2, 8)), fill=rng.randrange(35, 120))
    draw.rounded_rectangle((30, 20, 400, 140), radius=15, fill=25, outline=155, width=3)
    draw.ellipse((65, 35, 140, 110), fill=100)
    draw.polygon([(60, 135), (90, 95), (120, 95), (150, 135)], fill=135)
    draw.text((180, 45), text, font=ImageFont.truetype(font_path, 20), fill=245)
    image.save(root / f'{name}.png')
    (root / f'{name}.txt').write_text(text + '\n')
