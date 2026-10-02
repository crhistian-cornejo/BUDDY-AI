#!/usr/bin/env python3
"""Buddy's base character: a chibi in a mint mochi hood with a sprout, 48x48.

Writes core/characters/buddy-base.json (the source the core loads). With --preview DIR it also writes a PNG sheet
and one animated GIF per state. The JSON is what ships; this script is how it is drawn: every state is a list of
poses (eyes, arms, legs, mouth, bob, a symbol), so new states are added in STATES below. Needs Pillow.
"""
import json
import sys
from dataclasses import dataclass, replace
from pathlib import Path

N = 48
OY = 3  # the whole figure sits this many rows down, leaving room for the sprout
PAL = {
    ".": None,
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
    "y": "#FFD45C",  # symbols: ? and star
    "r": "#FF5D6C",  # symbols: !
}


@dataclass(frozen=True)
class Pose:
    eyes: str = "open"  # open closed left right up down happy wide x
    mouth: str = "smile"  # smile o flat
    head: int = 0  # head bob (rows down)
    lift: int = 0  # whole figure up (rows), for jumps
    shake: int = 0  # whole figure sideways (columns)
    left_arm: str = "down"  # down up
    right_arm: str = "down"  # down out up
    hands: int = 0  # typing: 1 lifts the left hand, 2 the right
    left_leg: int = 0  # rows lifted
    right_leg: int = 0
    left_step: int = 0  # columns the foot moves (walking stride)
    right_step: int = 0
    dangle: bool = False  # legs hang (being dragged)
    symbol: str = ""  # dots1 dots2 dots3 question question2 exclaim star


def rr(x, y, x0, y0, x1, y1, r):
    """Inside a rounded rect (inclusive pixel bounds)."""
    if not (x0 <= x <= x1 and y0 <= y <= y1):
        return False
    cx = min(max(x, x0 + r), x1 - r)
    cy = min(max(y, y0 + r), y1 - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r + r * 0.6


def shape(g, mask, fill, outline="o"):
    for y in range(N):
        for x in range(N):
            if not mask(x, y):
                continue
            edge = any(not mask(x + dx, y + dy) for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)))
            g[y][x] = outline if edge else fill(x, y)


def noise(x, y):
    return ((x * 73856093) ^ (y * 19349663)) % 11


def put(g, x, y, ch):
    if 0 <= x < N and 0 <= y < N:
        g[y][x] = ch


def outline_points(g, pts):
    for x, y in list(pts):
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            nx, ny = x + dx, y + dy
            if 0 <= nx < N and 0 <= ny < N and (nx, ny) not in pts and g[ny][nx] == ".":
                g[ny][nx] = "o"


def draw(p: Pose):
    g = [["."] * N for _ in range(N)]
    dy = p.head

    # legs (lifted ones are shorter; dangling ones hang lower and apart)
    for i, lx in enumerate((18, 26)):
        lift = p.left_leg if i == 0 else p.right_leg
        lx += p.left_step if i == 0 else p.right_step
        if p.dangle:
            lx += -1 if i == 0 else 1
        y1 = 43 - lift + (1 if p.dangle else 0)
        if lift:  # a lifted foot rises whole, so it still shows under the body
            shape(g, lambda x, y, lx=lx, y1=y1: rr(x, y, lx, 38 - lift, lx + 4, y1, 2),
                  lambda x, y, lx=lx, y1=y1: "d" if y >= y1 - 2 else ("B" if x == lx + 3 else "b"))
            continue
        shape(g, lambda x, y, lx=lx, y1=y1: rr(x, y, lx, 38, lx + 4, y1, 2),
              lambda x, y, lx=lx, y1=y1: "d" if y >= y1 - 2 else ("B" if x == lx + 3 else "b"))

    # arms resting at the sides (mint sleeve, cream hand); typing lifts a hand
    def side_arm(ax, lifted):
        y0, y1 = 31 - lifted, 37 - lifted
        shape(g, lambda x, y: rr(x, y, ax, y0, ax + 4, y1, 2),
              lambda x, y: ("S" if y >= y1 - 1 else "s") if y >= y1 - 2
              else ("B" if (x == ax + 3 if ax > 20 else x == ax + 1) else "b"))

    if p.left_arm == "down":
        side_arm(12, 1 if p.hands == 1 else 0)
    if p.right_arm == "down":
        side_arm(32, 1 if p.hands == 2 else 0)

    # body (mint onesie) with a cream belly
    shape(g, lambda x, y: rr(x, y, 15, 29, 32, 41, 4),
          lambda x, y: "d" if y >= 40 else ("B" if x >= 30 or y >= 38 else ("l" if x <= 16 else "b")))
    for y in range(33, 39):
        for x in range(19, 29):
            if rr(x, y, 19, 33, 28, 38, 3):
                g[y][x] = "S" if (y >= 37 or x >= 27) else "s"

    # hood (head), over the body; `head` bobs it down
    def hood(x, y):
        t = (x - 9) / 29 * 0.45 + (y - 8 - dy) / 23 * 0.9
        n = noise(x, y)
        tone = "l" if t < 0.28 else ("b" if t < 0.82 else ("B" if t < 1.12 else "d"))
        if n == 0 and tone == "b":
            tone = "l"
        if n == 1 and tone in "bl":
            tone = "B"
        return tone

    shape(g, lambda x, y: rr(x, y, 9, 8 + dy, 38, 31 + dy, 7), hood)
    for x, y in ((13, 12 + dy), (14, 12 + dy), (12, 13 + dy), (12, 14 + dy)):
        g[y][x] = "w"

    # face window: recessed rim, skin inside
    F = (15, 15 + dy, 32, 28 + dy, 4)
    for y in range(N):
        for x in range(N):
            if rr(x, y, *F):
                if not rr(x - 1, y, *F) or not rr(x, y - 1, *F):
                    g[y][x] = "d"
                elif not rr(x + 1, y, *F) or not rr(x, y + 1, *F):
                    g[y][x] = "B"
                else:
                    g[y][x] = "S" if (x >= F[2] - 2 or y >= F[3] - 1) else "s"

    draw_eyes(g, p.eyes, 19 + dy)
    ey = 19 + dy
    for bx in (17, 18, 29, 30):
        g[ey + 5][bx] = "c"
    for bx in (17, 30):
        g[ey + 4][bx] = "c"
    if p.mouth == "smile":
        g[ey + 5][23] = g[ey + 5][24] = "m"
    elif p.mouth == "o":
        for x, y in ((23, ey + 5), (24, ey + 5), (23, ey + 6), (24, ey + 6)):
            g[y][x] = "m"
    else:  # flat
        g[ey + 5][22] = g[ey + 5][23] = g[ey + 5][24] = g[ey + 5][25] = "m"

    # raised arms go outside the hood
    if p.left_arm == "up":
        raised_arm(g, 4, 20 + dy, mirror=True)
    if p.right_arm == "up":
        raised_arm(g, 39, 20 + dy)
    elif p.right_arm == "out":
        shape(g, lambda x, y: rr(x, y, 37, 26 + dy, 43, 31 + dy, 2),
              lambda x, y: "s" if x >= 41 else ("B" if y >= 30 + dy else "b"))

    draw_sprout(g, 8 + dy)

    # whole-figure moves, then the symbol (in final coordinates, top right)
    lift = OY - p.lift
    g = [["."] * N for _ in range(lift)] + g[: N - lift] if lift >= 0 else g[-lift:] + [["."] * N for _ in range(-lift)]
    if p.shake:
        g = [(row[-p.shake:] + row[:-p.shake]) if p.shake > 0 else (row[-p.shake:] + row[:-p.shake]) for row in g]
    draw_symbol(g, p.symbol)
    return ["".join(r) for r in g]


def raised_arm(g, ax, ay, mirror=False):
    """An arm up beside the head: sleeve below, cream hand on top."""
    shape(g, lambda x, y: rr(x, y, ax, ay, ax + 4, ay + 8, 2),
          lambda x, y: ("s" if y <= ay + 2 else ("B" if (x == ax + 1 if mirror else x == ax + 3) else "b")))


def draw_eyes(g, eyes, ey):
    for ex in (19, 27):
        if eyes in ("open", "left", "right", "up", "down", "wide"):
            ox = {"left": -1, "right": 1}.get(eyes, 0)
            oy = {"up": -1, "down": 1}.get(eyes, 0)
            height = 5 if eyes == "wide" else 4
            for y in range(ey + oy - (1 if eyes == "wide" else 0), ey + oy + height - (1 if eyes == "wide" else 0)):
                g[y][ex + ox] = g[y][ex + ox + 1] = "k"
            g[ey + oy - (1 if eyes == "wide" else 0)][ex + ox + (1 if eyes == "right" else 0)] = "w"
        elif eyes == "closed":
            for x in range(ex - 1, ex + 3):
                g[ey + 3][x] = "k"
        elif eyes == "happy":  # ^ ^
            for x, y in ((ex - 1, ey + 3), (ex, ey + 2), (ex + 1, ey + 2), (ex + 2, ey + 3)):
                g[y][x] = "k"
        elif eyes == "x":
            for d in range(4):
                g[ey + d][ex - 1 + d] = "k"
                g[ey + d][ex + 2 - d] = "k"


def draw_sprout(g, top):
    leaves = {
        "left": [(x, y) for y in range(top - 8, top - 2) for x in range(15, 24)
                 if abs(x - 19.0) <= 4.6 and abs(y - (top - 5.0)) <= 2.6 * (1 - ((x - 19.0) / 4.6) ** 2) + 0.4],
        "right": [(x, y) for y in range(top - 11, top - 2) for x in range(24, 34)
                  if abs(x - 28.5) <= 5.2 and abs(y - (top - 6.5)) <= 2.9 * (1 - ((x - 28.5) / 5.2) ** 2) + 0.4],
    }
    stem = {(23, y) for y in range(top - 3, top)} | {(24, y) for y in range(top - 3, top)}
    pts = set(p for v in leaves.values() for p in v) | stem
    for x, y in pts:
        if 0 <= y < N:
            g[y][x] = "g"
    for x, y in leaves["left"]:
        if y == top - 5 and 16 <= x <= 22:
            g[y][x] = "G"
        elif y <= top - 7:
            g[y][x] = "L"
    for x, y in leaves["right"]:
        if y == top - 6 and 25 <= x <= 32:
            g[y][x] = "G"
        elif y <= top - 8:
            put(g, x, y, "L")
    for y in range(top - 3, top + 1):
        g[y][23] = g[y][24] = "G"
    outline_points(g, {(x, y) for x, y in pts if 0 <= y < N})


GLYPHS = {
    # 5x7, drawn in the symbol color with an outline
    "question": [".###.", "#...#", "....#", "..##.", "..#..", ".....", "..#.."],
    "exclaim": ["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#.."],
    "star": [".....", "..#..", ".###.", "#####", ".###.", ".#.#.", "....."],
}


def draw_symbol(g, symbol):
    if not symbol:
        return
    if symbol.startswith("dots"):
        for i in range(int(symbol[-1])):
            x = 37 + i * 3
            for dx, dy in ((0, 0), (1, 0), (0, 1), (1, 1)):
                g[11 + dy][x + dx] = "o"
        return
    name = symbol.rstrip("2")
    color = "r" if name == "exclaim" else "y"
    top = 2 if symbol.endswith("2") else 3
    pts = set()
    for r, row in enumerate(GLYPHS[name]):
        for c, ch in enumerate(row):
            if ch == "#":
                pts.add((39 + c, top + r))
                g[top + r][39 + c] = color
    outline_points(g, pts)


I = Pose()
STATES = {
    # name: (fps, poses). Every state but idle is played by the app and then returns to idle.
    "idle": (2, [I, replace(I, head=1)]),
    "blink": (10, [replace(I, eyes="closed")]),
    "look": (2, [replace(I, eyes="left"), replace(I, eyes="left"), replace(I, eyes="right"), replace(I, eyes="right")]),
    "wave": (6, [
        replace(I, eyes="happy", right_arm="out"),
        replace(I, eyes="happy", right_arm="up"),
        replace(I, eyes="happy", right_arm="out"),
        replace(I, eyes="happy", right_arm="up"),
        replace(I, eyes="happy", right_arm="out"),
        replace(I, eyes="happy", right_arm="up"),
    ]),
    "walk-right": (8, [
        replace(I, eyes="right", lift=1, left_leg=1, left_step=2, right_step=-1),
        replace(I, eyes="right", head=1),
        replace(I, eyes="right", lift=1, right_leg=1, right_step=2, left_step=-1),
        replace(I, eyes="right", head=1),
    ]),
    "walk-left": (8, [
        replace(I, eyes="left", lift=1, right_leg=1, right_step=-2, left_step=1),
        replace(I, eyes="left", head=1),
        replace(I, eyes="left", lift=1, left_leg=1, left_step=-2, right_step=1),
        replace(I, eyes="left", head=1),
    ]),
    "drag": (4, [
        replace(I, eyes="wide", mouth="o", left_arm="up", right_arm="up", dangle=True),
        replace(I, eyes="wide", mouth="o", left_arm="up", right_arm="up", dangle=True, head=1),
    ]),
    "think": (3, [
        replace(I, eyes="up", mouth="flat", symbol="dots1"),
        replace(I, eyes="up", mouth="flat", symbol="dots2"),
        replace(I, eyes="up", mouth="flat", symbol="dots3"),
        replace(I, eyes="up", mouth="flat"),
    ]),
    "work": (8, [
        replace(I, eyes="down", mouth="flat", hands=1),
        replace(I, eyes="down", mouth="flat"),
        replace(I, eyes="down", mouth="flat", hands=2),
        replace(I, eyes="down", mouth="flat"),
    ]),
    "ask": (4, [
        replace(I, eyes="open", symbol="question"),
        replace(I, eyes="open", symbol="question2"),
        replace(I, eyes="open", head=1, symbol="question2"),
        replace(I, eyes="open", head=1, symbol="question"),
    ]),
    "error": (8, [
        replace(I, eyes="x", mouth="flat", symbol="exclaim", shake=1),
        replace(I, eyes="x", mouth="flat", symbol="exclaim", shake=-1),
        replace(I, eyes="x", mouth="flat", symbol="exclaim"),
    ]),
    "done": (8, [
        replace(I, eyes="happy", mouth="o", head=1),
        replace(I, eyes="happy", mouth="o", lift=2, left_arm="up", right_arm="up", symbol="star"),
        replace(I, eyes="happy", mouth="o", lift=3, left_arm="up", right_arm="up", symbol="star"),
        replace(I, eyes="happy", mouth="o", lift=2, left_arm="up", right_arm="up", symbol="star"),
        replace(I, eyes="happy", symbol="star"),
    ]),
    "sleep": (1, [
        replace(I, eyes="closed", mouth="flat"),
        replace(I, eyes="closed", mouth="flat", head=1),
    ]),
}


def main():
    states = {name: {"fps": fps, "frames": [draw(p) for p in poses]} for name, (fps, poses) in STATES.items()}
    # The square the apps crop as Buddy's avatar (hood, face and sprout), in idle frame 0.
    face = {"x": 7, "y": 1, "size": 34}
    data = {"id": "buddy-base", "name": "Buddy", "size": N, "face": face, "palette": PAL, "states": states}
    root = Path(__file__).resolve().parents[2]
    out = root / "core/characters/buddy-base.json"
    out.write_text(json.dumps(data, indent=1, ensure_ascii=False) + "\n")
    print(f"→ {out}")
    if "--preview" in sys.argv:
        preview(states, face, Path(sys.argv[sys.argv.index("--preview") + 1]))


def preview(states, face, folder):
    from PIL import Image, ImageDraw

    def render(frame, scale, bg=(250, 250, 250)):
        img = Image.new("RGB", (N * scale, N * scale), bg)
        d = ImageDraw.Draw(img)
        for r, row in enumerate(frame):
            for c, ch in enumerate(row):
                if PAL[ch]:
                    d.rectangle([c * scale, r * scale, c * scale + scale - 1, r * scale + scale - 1], fill=PAL[ch])
        return img

    folder.mkdir(parents=True, exist_ok=True)
    rows = list(states.items())
    cols = max(len(s["frames"]) for _, s in rows)
    scale = 4
    sheet = Image.new("RGB", (cols * N * scale + 140, len(rows) * N * scale), (250, 250, 250))
    d = ImageDraw.Draw(sheet)
    for i, (name, s) in enumerate(rows):
        d.text((6, i * N * scale + N * scale // 2), name, fill=(40, 40, 40))
        for j, f in enumerate(s["frames"]):
            sheet.paste(render(f, scale), (140 + j * N * scale, i * N * scale))
        frames = [render(f, 6) for f in s["frames"]]
        frames[0].save(folder / f"{name}.gif", save_all=True, append_images=frames[1:],
                       duration=int(1000 / s["fps"]), loop=0)
    sheet.save(folder / "sheet.png")
    f = render(states["idle"]["frames"][0], 8)
    f.crop((face["x"] * 8, face["y"] * 8, (face["x"] + face["size"]) * 8, (face["y"] + face["size"]) * 8)).save(folder / "face.png")


if __name__ == "__main__":
    main()
