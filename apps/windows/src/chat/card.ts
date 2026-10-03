// One card of an answer (see core/src/cards.rs): title, weather, figures, chart, table, steps and follow-ups, in that
// order, each only when the card has it. Built with DOM and SVG calls only (text through textContent, no HTML
// parsed). Measures and chart colours come from assets/design-tokens.json (`card`, `chart`): the same file the Mac
// app reads, so a card is the same card on both. Twin of Sources/Chat/CardView.swift on macOS.

import "./card.css";
import tokens from "../tokens";
import { h } from "./dom";
import { invoke } from "@tauri-apps/api/core";

export interface Metric { etiqueta: string; valor: string; nota: string; tono: string }
export interface Series { nombre: string; valores: number[] }
export interface Chart { tipo: string; etiquetas: string[]; series: Series[]; unidad: string }
export interface Table { columnas: string[]; filas: string[][] }
export interface WeatherDay { dia: string; icono: string; max: number; min: number; lluvia: number }
export interface Weather { lugar: string; temperatura: number; sensacion: number; estado: string; icono: string; noche: boolean; max: number; min: number; humedad: number; viento: number; lluvia: number; dias: WeatherDay[] }
export interface Card { titulo: string; subtitulo: string; metricas: Metric[]; grafico: Chart | null; tabla: Table | null; pasos: string[]; clima: Weather | null; fuente: string; modelo: string; acciones: string[] }

/** A picture made of the core's shapes (`cards::Backdrop`): the weather's sky. */
interface BackdropShape { kind: string; x: number; y: number; w: number; h: number; points: number[]; color: string; color2: string; opacity: number; size: number }
interface Backdrop { width: number; height: number; shapes: BackdropShape[] }

const FENCE = "```buddy-ui";
const NS = "http://www.w3.org/2000/svg";
const T = tokens.card;

/** The answer's text without its card blocks, the blocks' JSON in order, and the JSON so far of one that is still
 *  arriving (`null`: none is). Same cuts as the core's `message_parts`. */
export function splitCards(text: string): { text: string; blocks: string[]; partial: string | null } {
  // «```bu»: from its first characters, so the fence never shows as text while it is typed.
  if (!text.includes("```bu")) return { text, blocks: [], partial: null };
  const prose: string[] = [];
  const blocks: string[] = [];
  let block: string[] | null = null;
  const lines = text.split("\n");
  lines.forEach((line, i) => {
    const bare = line.trim();
    if (block === null) {
      const typing = i === lines.length - 1 && bare.length >= 5 && FENCE.startsWith(bare);
      if ((bare.startsWith("```") && bare.slice(3).trim() === "buddy-ui") || typing) block = [];
      else prose.push(line);
    } else if (bare === "```") {
      blocks.push(block.join("\n"));
      block = null;
    } else block.push(line);
  });
  return { text: prose.join("\n").trim(), blocks, partial: block === null ? null : (block as string[]).join("\n") };
}

const checked = new Map<string, Promise<Card | null>>();

/** The core checks every block (bounds, shapes): what it refuses is not drawn. */
export function cardFrom(json: string): Promise<Card | null> {
  let card = checked.get(json);
  if (!card) {
    card = invoke<Partial<Card> | null>("card_from_json", { json }).then((c) => c && complete(c)).catch(() => null);
    checked.set(json, card);
    if (checked.size > 60) checked.delete(checked.keys().next().value as string);
  }
  return card;
}

/** Every field present, whatever came (the core always sends them all). */
export function complete(card: Partial<Card>): Card {
  const weather = card.clima ? { ...card.clima, noche: !!card.clima.noche, lluvia: card.clima.lluvia ?? 0, dias: (card.clima.dias ?? []).map((d) => ({ ...d, lluvia: d.lluvia ?? 0 })) } : null;
  return {
    titulo: card.titulo ?? "", subtitulo: card.subtitulo ?? "", metricas: card.metricas ?? [], grafico: card.grafico ?? null, tabla: card.tabla ?? null,
    pasos: card.pasos ?? [], clima: weather, fuente: card.fuente ?? "", modelo: card.modelo ?? "", acciones: card.acciones ?? [],
  };
}

/** What a card has so far, from the JSON written until now (the core reads it; `null`: nothing to draw yet). */
export function cardPartial(json: string): Promise<Card | null> {
  if (!json.trim()) return Promise.resolve(null);
  return invoke<Partial<Card> | null>("card_partial", { json }).then((c) => c && complete(c)).catch(() => null);
}

function el(tag: string, attrs: Record<string, string | number>, ...children: Node[]): SVGElement {
  const node = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  node.append(...children);
  return node;
}

function seriesColor(index: number): string {
  return `var(--chart-${index % tokens.chart.dark.length})`;
}

/** «S/. 1,224» or «45 %»: a currency sign goes before the figure, any other unit after it. */
function figure(value: number, unit: string): string {
  const number = value.toLocaleString("en-US", { maximumFractionDigits: 1 });
  if (!unit) return number;
  return /[$€]|S\//.test(unit) ? `${unit} ${number}` : `${number} ${unit}`;
}

// Stroke icons on a 24 grid, one per weather picture the core knows (`cards::ICONS`).
const CLOUD = "M7 18a4 4 0 0 1-.6-7.96A5.5 5.5 0 0 1 17 9.5h.5a4.25 4.25 0 0 1 0 8.5z";
const WEATHER: Record<string, string> = {
  sol: "M12 8a4 4 0 1 0 0 8a4 4 0 0 0 0-8zM12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41",
  parcial: "M8 20a4 4 0 0 1-.6-7.96A5.5 5.5 0 0 1 17.5 12h.25a4 4 0 0 1 0 8zM16 3v1.5M21.5 8.5H20M20 4.5l-1 1M12.3 6.6A4 4 0 0 1 19.4 10",
  nubes: CLOUD,
  niebla: "M7 14a4 4 0 0 1-.6-7.96A5.5 5.5 0 0 1 17 5.5h.5a4.25 4.25 0 0 1 0 8.5zM5 18h14M7 21h10",
  lluvia: "M7 15a4 4 0 0 1-.6-7.96A5.5 5.5 0 0 1 17 6.5h.5a4.25 4.25 0 0 1 0 8.5zM8 18l-1 3M12 18l-1 3M16 18l-1 3",
  tormenta: "M7 15a4 4 0 0 1-.6-7.96A5.5 5.5 0 0 1 17 6.5h.5a4.25 4.25 0 0 1 0 8.5zM13 14l-3 4h4l-3 4",
  nieve: "M7 15a4 4 0 0 1-.6-7.96A5.5 5.5 0 0 1 17 6.5h.5a4.25 4.25 0 0 1 0 8.5zM8 19h.01M12 19h.01M16 19h.01M10 22h.01M14 22h.01",
};

function weatherIcon(name: string, size: number): SVGElement {
  return el("svg", { viewBox: "0 0 24 24", width: size, height: size, fill: "none", stroke: "currentColor", "stroke-width": 1.5, "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true" },
    el("path", { d: WEATHER[name] ?? CLOUD }));
}

const round = (v: number) => String(Math.round(v));
const backdrops = new Map<string, Promise<Backdrop | null>>();
let gradients = 0;

/** The sky for a weather picture, as the core draws it (the same shapes the Mac paints). */
function backdropFor(icon: string, night: boolean): Promise<Backdrop | null> {
  const key = `${icon}|${night}`;
  let backdrop = backdrops.get(key);
  if (!backdrop) {
    backdrop = invoke<Backdrop | null>("weather_backdrop", { icon, night }).catch(() => null);
    backdrops.set(key, backdrop);
  }
  return backdrop;
}

/** Paints a backdrop: it covers the space and keeps its right edge. Twin of `BackdropView` on macOS. */
export function backdropView(backdrop: Backdrop): SVGElement {
  const root = el("svg", { viewBox: `0 0 ${backdrop.width} ${backdrop.height}`, preserveAspectRatio: "xMaxYMid slice", class: "ui-backdrop", "aria-hidden": "true" });
  for (const shape of backdrop.shapes) {
    let node: SVGElement;
    if (shape.kind === "ellipse") {
      node = el("ellipse", { cx: shape.x + shape.w / 2, cy: shape.y + shape.h / 2, rx: shape.w / 2, ry: shape.h / 2, fill: shape.color });
    } else if (shape.kind === "line") {
      node = el("line", { x1: shape.x, y1: shape.y, x2: shape.x + shape.w, y2: shape.y + shape.h, stroke: shape.color, "stroke-width": shape.size, "stroke-linecap": "round" });
    } else if (shape.kind === "poly") {
      const points: string[] = [];
      for (let i = 0; i + 1 < shape.points.length; i += 2) points.push(`${shape.points[i]},${shape.points[i + 1]}`);
      node = el("polygon", { points: points.join(" "), fill: shape.color });
    } else {
      let fill = shape.color;
      if (shape.color2) {
        const id = `ui-sky-${gradients++}`;
        root.append(el("defs", {}, el("linearGradient", { id, x1: 0, y1: 0, x2: 0, y2: 1 },
          el("stop", { offset: 0, "stop-color": shape.color }), el("stop", { offset: 1, "stop-color": shape.color2 }))));
        fill = `url(#${id})`;
      }
      node = el("rect", { x: shape.x, y: shape.y, width: shape.w, height: shape.h, rx: shape.size, fill });
    }
    if (shape.opacity < 1) node.setAttribute("opacity", String(shape.opacity));
    root.append(node);
  }
  return root;
}

/** The top of a weather card: the place, the temperature and the sky's words, over that sky. */
function weatherHero(w: Weather): HTMLElement {
  const hero = h("div", { class: "ui-hero-banner" },
    h("div", { class: "ui-hero-text" },
      h("div", { class: "ui-weather-place", text: w.lugar }),
      h("div", { class: "ui-hero", text: `${round(w.temperatura)}°` }),
      h("div", { class: "ui-weather-condition", text: `${w.estado} · ↑${round(w.max)}° ↓${round(w.min)}°` })));
  void backdropFor(w.icono, w.noche).then((backdrop) => { if (backdrop) hero.prepend(backdropView(backdrop)); });
  return hero;
}

/** Under the sky: today's facts and the next days. */
function weatherDetails(w: Weather): HTMLElement {
  const fact = (label: string, value: string) => h("div", { class: "ui-fact" }, h("span", { class: "ui-fact-label", text: label }), h("span", { class: "ui-fact-value", text: value }));
  const panel = h("div", { class: "ui-weather" },
    h("div", { class: "ui-facts" },
      fact("Sensación", `${round(w.sensacion)}°`), fact("Humedad", `${round(w.humedad)} %`),
      fact("Viento", `${round(w.viento)} km/h`), fact("Lluvia", `${round(w.lluvia)} %`)));
  if (w.dias.length) {
    // The chance of rain shows under each day only when some day is worth it.
    const showsRain = w.dias.some((d) => d.lluvia >= 30);
    panel.append(h("div", { class: "ui-rule" }), h("div", { class: "ui-days" }, ...w.dias.map((d) =>
      h("div", { class: "ui-day" },
        h("span", { class: "ui-day-label", text: d.dia }), weatherIcon(d.icono, 17),
        h("span", { class: "ui-day-high", text: `${round(d.max)}°` }), h("span", { class: "ui-day-low", text: `${round(d.min)}°` }),
        showsRain ? h("span", { class: "ui-day-rain", text: d.lluvia >= 30 ? `${round(d.lluvia)} %` : "\u00a0" }) : null))));
  }
  return panel;
}

/** Round figures for the axis: 4 steps from 0 (or the lowest value) to a little above the highest. */
function scale(values: number[]): { min: number; max: number; ticks: number[] } {
  const low = Math.min(0, ...values);
  const high = Math.max(0, ...values);
  const span = high - low || 1;
  const rough = span / 4;
  const power = 10 ** Math.floor(Math.log10(rough));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * power).find((s) => s >= rough) ?? rough;
  const min = Math.floor(low / step) * step;
  const max = Math.ceil(high / step) * step;
  const ticks: number[] = [];
  for (let v = min; v <= max + step / 2; v += step) ticks.push(Number(v.toFixed(6)));
  return { min, max, ticks };
}

function plot(chart: Chart): SVGElement {
  const width = T.maxWidth - 2 * T.padding;
  const height = T.chartHeight;
  const left = 34, bottom = 18, top = 6;
  const all = chart.series.flatMap((s) => s.valores);
  const { min, max, ticks } = scale(all);
  const y = (v: number) => top + (height - top - bottom) * (1 - (v - min) / (max - min || 1));
  const band = (width - left) / chart.etiquetas.length;
  const root = el("svg", { viewBox: `0 0 ${width} ${height}`, width: "100%", height, role: "img", "aria-label": `Gráfico ${chart.unidad}`.trim() });
  for (const t of ticks) {
    root.append(el("line", { x1: left, x2: width, y1: y(t), y2: y(t), class: "ui-grid" }));
    const label = el("text", { x: left - 6, y: y(t) + 3, "text-anchor": "end", class: "ui-axis" });
    label.textContent = Math.abs(t) >= 1000 ? `${(t / 1000).toLocaleString("en-US", { maximumFractionDigits: 1 })}k` : String(t);
    root.append(label);
  }
  // Fewer names than points when they would overlap.
  const every = Math.ceil(chart.etiquetas.length / Math.max(1, Math.floor((width - left) / 46)));
  chart.etiquetas.forEach((name, i) => {
    if (i % every) return;
    const label = el("text", { x: left + band * (i + 0.5), y: height - 4, "text-anchor": "middle", class: "ui-axis" });
    label.textContent = name.length > 9 ? `${name.slice(0, 8)}…` : name;
    root.append(label);
  });
  chart.series.forEach((series, s) => {
    if (chart.tipo === "lineas") {
      const points = series.valores.map((v, i) => `${left + band * (i + 0.5)},${y(v)}`).join(" ");
      root.append(el("polyline", { points, fill: "none", stroke: seriesColor(s), "stroke-width": 2, "stroke-linejoin": "round", "stroke-linecap": "round" }));
    } else {
      const inner = band * 0.7 / chart.series.length;
      series.valores.forEach((v, i) => {
        const x = left + band * i + band * 0.15 + inner * s;
        const bar = el("rect", { x, y: Math.min(y(v), y(0)), width: Math.max(inner - 2, 1), height: Math.max(Math.abs(y(v) - y(0)), 1), rx: 3, fill: seriesColor(s) });
        const title = document.createElementNS(NS, "title");
        title.textContent = `${chart.etiquetas[i]}: ${figure(v, chart.unidad)}`;
        bar.append(title);
        root.append(bar);
      });
    }
  });
  return root;
}

function pie(chart: Chart): HTMLElement {
  const values = chart.series[0]?.valores ?? [];
  const total = values.reduce((a, b) => a + b, 0) || 1;
  const size = T.chartHeight - 30;
  const r = size / 2, inner = r * 0.58;
  const root = el("svg", { viewBox: `0 0 ${size} ${size}`, width: size, height: size, role: "img", "aria-label": "Gráfico de torta" });
  let angle = -Math.PI / 2;
  const at = (a: number, radius: number) => `${r + radius * Math.cos(a)} ${r + radius * Math.sin(a)}`;
  values.forEach((v, i) => {
    if (v <= 0) return;
    const sweep = Math.min((v / total) * 2 * Math.PI, 2 * Math.PI - 0.0001);
    const end = angle + sweep;
    const large = sweep > Math.PI ? 1 : 0;
    root.append(el("path", {
      d: `M ${at(angle, r)} A ${r} ${r} 0 ${large} 1 ${at(end, r)} L ${at(end, inner)} A ${inner} ${inner} 0 ${large} 0 ${at(angle, inner)} Z`,
      fill: seriesColor(i), stroke: "var(--color-surface)", "stroke-width": 1,
    }));
    angle = end;
  });
  const legend = h("div", { class: "ui-pie-legend" }, ...chart.etiquetas.map((name, i) =>
    h("div", { class: "ui-legend-row" },
      h("i", { class: "ui-dot", style: `background:${seriesColor(i)}` }), h("span", { class: "ui-legend-name", text: name }),
      h("span", { class: "ui-legend-value", text: figure(values[i] ?? 0, chart.unidad) }))));
  const wrap = h("div", { class: "ui-pie" });
  wrap.append(root, legend);
  return wrap;
}

function chartPanel(chart: Chart): HTMLElement {
  const panel = h("div", { class: "ui-chart" });
  if (chart.tipo === "torta") return pie(chart);
  panel.append(plot(chart));
  if (chart.series.length > 1) {
    panel.append(h("div", { class: "ui-legend" }, ...chart.series.map((s, i) =>
      h("span", { class: "ui-legend-item" }, h("i", { class: "ui-dot", style: `background:${seriesColor(i)}` }), h("span", { text: s.nombre })))));
  }
  return panel;
}

/** How a card is being shown: `drawing` while it is still arriving (a skeleton stands for the rest), and `seen`,
 *  the parts already on screen, so that only what is new comes in with the entrance. */
export interface CardShow { drawing?: boolean; seen?: Set<string> }

/** The card. `send` runs a follow-up button (it sends that message for the user); without it there are no buttons. */
export function cardView(card: Card, send?: (text: string) => void, show: CardShow = {}): HTMLElement {
  // A part that was not there before comes in softly. Twin of `CardView.enter` on macOS.
  const part = <T extends HTMLElement>(id: string, node: T): T => {
    if (show.seen && !show.seen.has(id)) {
      show.seen.add(id);
      node.classList.add("ui-enter");
    }
    return node;
  };
  let root: HTMLElement = h("section", { class: "ui-card" });
  const vars: Record<string, string | number> = {
    "--ui-padding": `${T.padding}px`, "--ui-gap": `${T.gap}px`, "--ui-radius": `${T.radius}px`, "--ui-max": `${T.maxWidth}px`,
    "--ui-title": `${T.titleSize}px`, "--ui-label": `${T.labelSize}px`, "--ui-body": `${T.bodySize}px`,
    "--ui-metric": `${T.metricSize}px`, "--ui-hero": `${T.heroSize}px`, "--ui-action": `${T.actionHeight}px`, "--ui-scene": `${T.sceneHeight}px`,
  };
  for (const [k, v] of Object.entries(vars)) root.style.setProperty(k, String(v));
  if (card.clima) root.append(part("hero", weatherHero(card.clima)));
  const outer = root;
  root = h("div", { class: "ui-body" });
  outer.append(root);
  if (card.titulo || card.subtitulo) {
    root.append(part("header", h("div", { class: "ui-header" },
      card.titulo ? h("div", { class: "ui-title", text: card.titulo }) : null,
      card.subtitulo ? h("div", { class: "ui-subtitle", text: card.subtitulo }) : null)));
  }
  if (card.clima) root.append(part("weather", weatherDetails(card.clima)));
  if (card.metricas.length) {
    root.append(part("metrics", h("div", { class: "ui-metrics", style: `grid-template-columns: repeat(${Math.min(card.metricas.length, 3)}, minmax(0, 1fr))` },
      ...card.metricas.map((m) => h("div", { class: "ui-metric" },
        h("span", { class: "ui-metric-label", text: m.etiqueta }),
        h("span", { class: "ui-metric-value", text: m.valor }),
        m.nota ? h("span", { class: `ui-metric-note ${m.tono}`.trim(), text: m.nota }) : null)))));
  }
  if (card.grafico) root.append(part("chart", chartPanel(card.grafico)));
  if (card.tabla) {
    root.append(part("table", h("table", { class: "ui-table" },
      h("thead", {}, h("tr", {}, ...card.tabla.columnas.map((c) => h("th", { text: c })))),
      h("tbody", {}, ...card.tabla.filas.map((row) => h("tr", {}, ...row.map((cell) => h("td", { text: cell }))))))));
  }
  if (card.pasos.length) {
    root.append(part("steps", h("ol", { class: "ui-steps" }, ...card.pasos.map((step, i) =>
      h("li", {}, h("span", { class: "ui-step-number", text: String(i + 1) }), h("span", { class: "ui-step-text", text: step }))))));
  }
  // Still arriving: two grey lines say that more is on its way.
  if (show.drawing) root.append(h("div", { class: "ui-skel-lines", "aria-hidden": "true" }, skel(180, 9), skel(110, 9)));
  if (card.acciones.length && send && !show.drawing) {
    root.append(part("actions", h("div", { class: "ui-actions" }, ...card.acciones.map((action) => {
      const button = h("button", { class: "ui-action", type: "button", title: `Enviar: ${action}`, text: action });
      button.addEventListener("click", () => send(action));
      return button;
    }))));
  }
  // Where the data comes from, and the model that drew the card (when one did).
  if (card.fuente || card.modelo) {
    root.append(part("footer", h("div", { class: "ui-footer" },
      h("span", { class: "ui-source", text: card.fuente ? `Fuente: ${card.fuente}` : "" }),
      card.modelo ? h("span", { class: "ui-drawn-by", text: card.modelo, title: `Tarjeta dibujada por ${card.modelo}` }) : null)));
  }
  return outer;
}

/** A grey block that stands for something still to come (it breathes while it waits). */
function skel(width: number | null, height: number): HTMLElement {
  return h("span", { class: "ui-skel", style: `${width === null ? "flex:1;" : `width:${width}px;`}height:${height}px` });
}

/** A card that has opened but has nothing to show yet: a title, three figures and a chart in grey blocks, so it is
 *  clear that something is being drawn. Twin of `CardSkeleton` on macOS. */
export function cardSkeleton(): HTMLElement {
  const root = h("section", { class: "ui-card", role: "status", "aria-label": "Dibujando la tarjeta" });
  root.style.setProperty("--ui-padding", `${T.padding}px`);
  root.style.setProperty("--ui-gap", `${T.gap}px`);
  root.style.setProperty("--ui-radius", `${T.radius}px`);
  root.style.setProperty("--ui-max", `${T.maxWidth}px`);
  root.append(h("div", { class: "ui-body" },
    h("div", { class: "ui-skel-lines" }, skel(150, 12), skel(100, 9)),
    h("div", { class: "ui-skel-metrics" }, ...[0, 1, 2].map(() => h("div", { class: "ui-skel-metric" }, skel(44, 8), skel(72, 16)))),
    h("div", { class: "ui-skel-bars" }, ...[0.45, 0.8, 0.6, 1, 0.4, 0.7].map((part) => skel(null, 84 * part)))));
  return root;
}

/** Chart colours as CSS variables (--chart-0…), following the theme like the rest of the tokens. */
export function applyChartColors(root: HTMLElement = document.documentElement): void {
  const dark = window.matchMedia("(prefers-color-scheme: dark)");
  const paint = () => (dark.matches ? tokens.chart.dark : tokens.chart.light).forEach((c, i) => root.style.setProperty(`--chart-${i}`, c));
  paint();
  dark.addEventListener("change", paint);
}
