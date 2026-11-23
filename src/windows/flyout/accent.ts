import { hslToRgb, rgbToHsl, type Rgb } from "../../lib/color";

const cache = new Map<string, Rgb | null>();

/** Kapaktan vurgu rengi: küçük bir tuvalde doygunluğa göre ağırlıklı ortalama. */
export function artAccent(src: string | null | undefined): Promise<Rgb | null> {
  if (!src) return Promise.resolve(null);
  if (cache.has(src)) return Promise.resolve(cache.get(src)!);
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      let out: Rgb | null = null;
      try {
        const c = document.createElement("canvas");
        c.width = c.height = 16;
        const ctx = c.getContext("2d", { willReadFrequently: true })!;
        ctx.drawImage(img, 0, 0, 16, 16);
        const d = ctx.getImageData(0, 0, 16, 16).data;
        let r = 0,
          g = 0,
          b = 0,
          w = 0;
        for (let i = 0; i < d.length; i += 4) {
          if (d[i + 3] < 128) continue;
          const [, s, l] = rgbToHsl([d[i], d[i + 1], d[i + 2]]);
          const wt = 0.05 + s * (1 - Math.abs(l - 0.5) * 1.6);
          r += d[i] * wt;
          g += d[i + 1] * wt;
          b += d[i + 2] * wt;
          w += wt;
        }
        if (w > 0) {
          const [h, s, l] = rgbToHsl([r / w, g / w, b / w]);
          // Koyu cam üstünde okunur: ışık 0.5–0.72, doygunluk en az 0.35.
          out = hslToRgb([h, Math.max(s, 0.35), Math.min(0.72, Math.max(0.5, l))]);
        }
      } catch {
        out = null;
      }
      cache.set(src, out);
      resolve(out);
    };
    img.onerror = () => resolve(null);
    img.src = src;
  });
}
