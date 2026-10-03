// Development only (not built): what the core's `cards::repaired` does, for the browser preview, where there is no
// core. In the app the core reads a card that is still arriving (`card_partial`); this is the same reading, ported.
window.previewPartialCard = function (json) {
  const text = json.trimStart();
  const closers = (stack) => [...stack].reverse().map((open) => (open === "{" ? "}" : "]")).join("");
  const stack = [];
  let safe = [0, []];
  let next = "value";
  let i = 0;
  let fixed = null;
  const finish = () => (safe[0] > 0 ? text.slice(0, safe[0]) + closers(safe[1]) : null);
  scan: {
    while (i < text.length) {
      const c = text[i];
      if (" \n\r\t".includes(c)) { i++; continue; }
      if ((c === "{" || c === "[") && next === "value") {
        stack.push(c); next = c === "{" ? "key" : "value"; i++; safe = [i, [...stack]]; continue;
      }
      if (c === "}" || c === "]") {
        if (stack[stack.length - 1] !== (c === "}" ? "{" : "[") || next === "colon") { fixed = finish(); break scan; }
        stack.pop(); next = "comma"; i++;
        if (!stack.length) { fixed = text.slice(0, i); break scan; }
        safe = [i, [...stack]]; continue;
      }
      if (c === '"' && (next === "key" || next === "value")) {
        let whole = i + 1;
        let closed = false;
        i++;
        while (i < text.length) {
          if (text[i] === '"') { closed = true; i++; break; }
          if (text[i] === "\\") {
            const needs = text[i + 1] === "u" ? 6 : 2;
            if (i + needs > text.length) { i = text.length; break; }
            i += needs; whole = i;
          } else { i++; whole = i; }
        }
        if (!closed) { fixed = next === "key" ? finish() : text.slice(0, whole) + '"' + closers(stack); break scan; }
        if (next === "key") next = "colon"; else { next = "comma"; safe = [i, [...stack]]; }
        continue;
      }
      if (c === ":" && next === "colon") { next = "value"; i++; continue; }
      if (c === "," && next === "comma" && stack.length) { next = stack[stack.length - 1] === "{" ? "key" : "value"; i++; continue; }
      if (next === "value") {
        const start = i;
        while (i < text.length && !',}] \n\r\t:"{['.includes(text[i])) i++;
        const token = text.slice(start, i);
        const word = ["true", "false", "null"].includes(token);
        if (i === text.length) {
          const digits = token.replace(/[^0-9]+$/, "");
          if (word) fixed = text + closers(stack);
          else if (digits && Number.isFinite(Number(digits))) fixed = text.slice(0, start + digits.length) + closers(stack);
          else fixed = finish();
          break scan;
        }
        if (!word && !Number.isFinite(Number(token))) { fixed = finish(); break scan; }
        next = "comma"; safe = [i, [...stack]]; continue;
      }
      fixed = finish(); break scan;
    }
    fixed = finish();
  }
  try {
    const card = fixed && JSON.parse(fixed);
    if (!card || typeof card !== "object" || Array.isArray(card)) return null;
    // Buttons and the footer belong to a finished card.
    delete card.acciones; delete card.fuente; delete card.modelo;
    // Like the core: a chart needs a value to be one, and its series are as long as its labels.
    const chart = card.grafico;
    if (chart) {
      const labels = chart.etiquetas ?? [];
      chart.etiquetas = labels; chart.unidad = chart.unidad ?? ""; chart.tipo = chart.tipo === "lineas" || chart.tipo === "torta" ? chart.tipo : "barras";
      chart.series = (chart.series ?? []).map((s) => ({ nombre: s.nombre ?? "", valores: labels.map((_, n) => Number(s.valores?.[n] ?? 0)) }));
      if (!labels.length || !chart.series.some((s) => s.valores.some((v) => v !== 0))) delete card.grafico;
    }
    if (card.tabla && (!card.tabla.columnas?.length || !card.tabla.filas?.length)) delete card.tabla;
    if (card.tabla) card.tabla.filas = card.tabla.filas.map((row) => card.tabla.columnas.map((_, n) => String(row?.[n] ?? "")));
    if (card.metricas) card.metricas = card.metricas.filter((m) => m && m.valor).map((m) => ({ etiqueta: m.etiqueta ?? "", valor: String(m.valor), nota: m.nota ?? "", tono: m.tono ?? "" }));
    const something = card.titulo || card.subtitulo || card.metricas?.length || card.grafico || card.tabla || card.pasos?.length || card.clima;
    return something ? card : null;
  } catch {
    return null;
  }
};
