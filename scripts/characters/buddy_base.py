#!/usr/bin/env python3
"""Buddy's base character ("Mochi"): a chibi in a mint mochi hood with a sprout, 48x48.

Writes core/characters/buddy-base.json (the source the core loads) and, with --preview DIR, PNG previews.
The JSON is what ships; this script is how it is drawn, so new states are added here. Needs Pillow.
"""
import json, sys
from PIL import Image
N = 48
OY = 3
PAL = {".": None,
  "o": "#1E4D3C",  # outline
  "d": "#3E9474",  # deep shade
  "B": "#5FBF98",  # shade
  "b": "#86DDB8",  # mint
  "l": "#B9F0D6",  # light
  "s": "#FCE6CC",  # skin
  "S": "#EDC7A3",  # skin shade
  "k": "#1A1A22",  # eyes
  "w": "#FFFFFF",  # glint
  "c": "#FF9AAE",  # blush
  "m": "#B5735A",  # mouth
  "g": "#8ED66B",  # leaf
  "G": "#4E9E3C",  # leaf shade, stem
  "L": "#C8F29E",  # leaf light
}

def rr(x, y, x0, y0, x1, y1, r):
    """Inside a rounded rect (inclusive pixel bounds)."""
    if not (x0 <= x <= x1 and y0 <= y <= y1): return False
    cx = min(max(x, x0 + r), x1 - r); cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r + r * 0.6

def shape(g, mask, fill_fn, outline="o"):
    for y in range(N):
        for x in range(N):
            if not mask(x, y): continue
            edge = any(not mask(x + dx, y + dy) for dx, dy in ((1,0),(-1,0),(0,1),(0,-1)))
            g[y][x] = outline if edge else fill_fn(x, y)

def noise(x, y):
    return ((x * 73856093) ^ (y * 19349663)) % 11

def frame(dy=0, eyes="open"):
    g = [["."] * N for _ in range(N)]
    # legs
    for lx in (18, 26):
        shape(g, lambda x, y, lx=lx: rr(x, y, lx, 38, lx + 4, 43, 2),
              lambda x, y: "d" if y >= 41 else ("B" if x == lx + 3 else "b"))
    # arms (cream hands)
    for ax in (12, 32):
        shape(g, lambda x, y, ax=ax: rr(x, y, ax, 31, ax + 4, 37, 2),
              lambda x, y, ax=ax: ("S" if y >= 36 else "s") if y >= 35 else ("B" if (x == ax + 3 if ax > 20 else x == ax + 1) else "b"))
    # body (mint onesie) with a cream belly
    shape(g, lambda x, y: rr(x, y, 15, 29, 32, 41, 4),
          lambda x, y: "d" if y >= 40 else ("B" if x >= 30 or y >= 38 else ("l" if x <= 16 else "b")))
    for y in range(33, 39):
        for x in range(19, 29):
            if rr(x, y, 19, 33, 28, 38, 3): g[y][x] = "S" if (y >= 37 or x >= 27) else "s"
    # hood (head), drawn over the body; dy bobs it down for the breath
    H = (9, 8 + dy, 38, 31 + dy, 7)
    def hood(x, y):
        u = (x - 9) / 29; v = (y - 8 - dy) / 23
        t = u * 0.45 + v * 0.9
        n = noise(x, y)
        tone = "l" if t < 0.28 else ("b" if t < 0.82 else ("B" if t < 1.12 else "d"))
        if n == 0 and tone == "b": tone = "l"
        if n == 1 and tone in "bl": tone = "B"
        return tone
    shape(g, lambda x, y: rr(x, y, *H), hood)
    # glint on the hood
    for (x, y) in ((13, 12 + dy), (14, 12 + dy), (12, 13 + dy), (12, 14 + dy)): g[y][x] = "w"
    # face window: recessed (deep shade rim at top-left), skin inside
    F = (15, 15 + dy, 32, 28 + dy, 4)
    for y in range(N):
        for x in range(N):
            if rr(x, y, *F):
                inner = rr(x, y, F[0] + 1, F[1] + 1, F[2], F[3], F[4] - 1) if True else True
                edge_tl = not rr(x - 1, y, *F) or not rr(x, y - 1, *F)
                edge_br = not rr(x + 1, y, *F) or not rr(x, y + 1, *F)
                if edge_tl: g[y][x] = "d"
                elif edge_br: g[y][x] = "B"
                else: g[y][x] = "S" if (x >= F[2] - 2 or y >= F[3] - 1) else "s"
    ey = 19 + dy
    if eyes == "open":
        for ex in (19, 27):
            for y in range(ey, ey + 4):
                g[y][ex] = g[y][ex + 1] = "k"
            g[ey][ex] = "w"
    else:
        for ex in (19, 27):
            g[ey + 3][ex - 1] = g[ey + 3][ex] = g[ey + 3][ex + 1] = g[ey + 3][ex + 2] = "k"
    for bx in (17, 18, 29, 30):
        g[ey + 5][bx] = "c"
    for bx in (17, 30):
        g[ey + 4][bx] = "c"
    g[ey + 5][23] = g[ey + 5][24] = "m"
    # sprout: stem and two leaves
    top = 8 + dy
    for y in range(top - 3, top + 1): g[y][23] = "G"; g[y][24] = "G" if y < top else g[y][24]
    leaves = {
        "left":  [(x, y) for y in range(top - 8, top - 2) for x in range(15, 24)
                  if abs(x - 19.0) <= 4.6 and abs(y - (top - 5.0)) <= 2.6 * (1 - ((x - 19.0) / 4.6) ** 2) + 0.4],
        "right": [(x, y) for y in range(top - 11, top - 2) for x in range(24, 34)
                  if abs(x - 28.5) <= 5.2 and abs(y - (top - 6.5)) <= 2.9 * (1 - ((x - 28.5) / 5.2) ** 2) + 0.4],
    }
    pts = set(p for v in leaves.values() for p in v) | {(23, y) for y in range(top - 3, top)} | {(24, y) for y in range(top - 3, top)}
    for (x, y) in pts:
        if g[y][x] == "." or g[y][x] == "G":
            g[y][x] = "g"
    for (x, y) in leaves["left"]:
        if y == top - 5 and 16 <= x <= 22: g[y][x] = "G"      # vein
        elif y <= top - 7: g[y][x] = "L"
    for (x, y) in leaves["right"]:
        if y == top - 6 and 25 <= x <= 32: g[y][x] = "G"
        elif y <= top - 8: g[y][x] = "L"
    for y in range(top - 3, top + 1): g[y][23] = "G"; g[y][24] = "G"
    for (x, y) in list(pts):
        for ddx, ddy in ((1,0),(-1,0),(0,1),(0,-1)):
            nx, ny = x + ddx, y + ddy
            if 0 <= nx < N and 0 <= ny < N and (nx, ny) not in pts and g[ny][nx] == ".": g[ny][nx] = "o"
    g = [["."] * N for _ in range(OY)] + g[:N - OY]
    return ["".join(r) for r in g]

idle = [frame(0), frame(1)]
blink = frame(0, "closed")
data = {"id": "buddy-base", "name": "Buddy", "size": N, "palette": PAL,
        "states": {"idle": {"fps": 2, "frames": idle}}}
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
out = ROOT / "core/characters/buddy-base.json"
out.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
print(f"→ {out}")
if "--preview" not in sys.argv:
    sys.exit(0)
preview_dir = Path(sys.argv[sys.argv.index("--preview") + 1])
S = 8
img = Image.new("RGB", (3 * N * S + 80, N * S + 20), (250, 250, 250))
for i, f in enumerate(idle + [blink]):
    for r, row in enumerate(f):
        for c, ch in enumerate(row):
            if PAL[ch]:
                col = tuple(int(PAL[ch][j:j+2], 16) for j in (1, 3, 5))
                for y in range(S):
                    for x in range(S):
                        img.putpixel((10 + i * (N * S + 30) + c * S + x, 10 + r * S + y), col)
img.save(preview_dir / "buddy-base.png")
# real-size preview (×2)
small = Image.new("RGB", (N * 2 * 3 + 40, N * 2 + 20), (250, 250, 250))
for i, f in enumerate(idle + [blink]):
    for r, row in enumerate(f):
        for c, ch in enumerate(row):
            if PAL[ch]:
                col = tuple(int(PAL[ch][j:j+2], 16) for j in (1, 3, 5))
                for y in range(2):
                    for x in range(2):
                        small.putpixel((10 + i * (N * 2 + 10) + c * 2 + x, 10 + r * 2 + y), col)
small.save(preview_dir / "buddy-base-x2.png")
