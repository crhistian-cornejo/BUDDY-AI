#!/usr/bin/env python3
"""Renders the app icon from a core character (idle, frame 0) and writes the Tauri and Mac icon sets.

Usage: scripts/make-icons.py [character-id]   (needs Pillow; run after changing the character)
"""
import json, subprocess, sys
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
cid = sys.argv[1] if len(sys.argv) > 1 else "buddy-base"
char = json.loads((ROOT / "core/characters" / f"{cid}.json").read_text())
frame = char["states"]["idle"]["frames"][0]
size = char["size"]

# The sprite's visible box, centered on a dark rounded square (the island black of the design tokens).
tokens = json.loads((ROOT / "assets/design-tokens.json").read_text())
cells = [(r, c) for r, row in enumerate(frame) for c, ch in enumerate(row) if char["palette"].get(ch)]
r0, r1 = min(r for r, _ in cells), max(r for r, _ in cells)
c0, c1 = min(c for _, c in cells), max(c for _, c in cells)
W = 1024
img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([64, 64, W - 64, W - 64], radius=200, fill=tokens["color"]["surface"])
span = max(r1 - r0, c1 - c0) + 1
px = (W - 2 * 200) // span
ox = (W - (c1 - c0 + 1) * px) // 2
oy = (W - (r1 - r0 + 1) * px) // 2
for r, c in cells:
    x, y = ox + (c - c0) * px, oy + (r - r0) * px
    d.rectangle([x, y, x + px - 1, y + px - 1], fill=char["palette"][frame[r][c]])

out = ROOT / "assets/icon.png"
img.save(out)
subprocess.run(["npx", "tauri", "icon", str(out), "-o", str(ROOT / "apps/windows/src-tauri/icons")],
               cwd=ROOT / "apps/windows", check=True)
print(f"icons from {cid} → {out}")
