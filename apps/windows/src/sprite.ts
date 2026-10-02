// Sprites come from buddy-core already rasterized: each frame is size×size pixels as 0xAARRGGBB (0 = transparent),
// the very same numbers the Mac app paints.

export interface SpriteState {
  name: string;
  fps: number;
  frames: number[][];
}

export interface Sprite {
  id: string;
  name: string;
  size: number;
  states: SpriteState[];
}

/** 0xAARRGGBB words → RGBA bytes for ImageData. */
export function argbToRgba(pixels: readonly number[]): Uint8ClampedArray<ArrayBuffer> {
  const out = new Uint8ClampedArray(pixels.length * 4);
  pixels.forEach((argb, i) => {
    out[i * 4] = (argb >>> 16) & 0xff;
    out[i * 4 + 1] = (argb >>> 8) & 0xff;
    out[i * 4 + 2] = argb & 0xff;
    out[i * 4 + 3] = (argb >>> 24) & 0xff;
  });
  return out;
}
