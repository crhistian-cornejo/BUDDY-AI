// Site tiles for the sources row. Twin of SourceLogos.fallbackColor/fallbackInitial on the Mac (MIKA, MIT). Drawn
// marks for well-known sites are not ported yet: every site gets its letter tile.

export interface Logo {
  id: string;
  bg: string;
  shapes: { d: string; fill?: string; stroke?: string; w?: number }[];
}

export function logoFor(_host: string): Logo | null {
  return null;
}

const PALETTE = ["#5b8def", "#e0795c", "#58a97b", "#b07adb", "#d9a441", "#4aa3b5", "#d4607f", "#7a86c9"];

export function registrableDomain(host: string): string {
  const h = host.toLowerCase().replace(/^www\./, "");
  if (!h.includes(".") || /^[\d.]+$/.test(h) || h.includes(":")) return h;
  const parts = h.split(".");
  const [sld, tld] = [parts[parts.length - 2]!, parts[parts.length - 1]!];
  const twoLevel = parts.length >= 3 && tld.length === 2 && ["co", "com", "org", "net", "gov", "edu", "ac"].includes(sld);
  return parts.slice(twoLevel ? -3 : -2).join(".");
}

export function fallbackColor(host: string): string {
  let sum = 0;
  for (const ch of registrableDomain(host)) sum = (sum * 31 + ch.codePointAt(0)!) & 0xffff;
  return PALETTE[sum % PALETTE.length]!;
}

export function fallbackInitial(host: string): string {
  const first = [...registrableDomain(host)].find((c) => /[\p{L}\p{N}]/u.test(c));
  return first ? first.toUpperCase() : "•";
}
