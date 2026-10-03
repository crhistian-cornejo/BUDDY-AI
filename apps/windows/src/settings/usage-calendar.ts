import { h } from "../chat/dom";
import { k } from "./ui";

export interface TokenDay {
  date: string;
  provider: string;
  turns: number;
  input: number;
  output: number;
  cached: number;
}

const key = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;

/** Calendar cells use local dates and real recorded totals; future days stay hidden. */
export function usageCalendar(days: TokenDay[]): HTMLElement {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const start = new Date(today);
  start.setDate(start.getDate() - start.getDay() - 364);
  const dates = Array.from({ length: 371 }, (_, index) => {
    const date = new Date(start);
    date.setDate(start.getDate() + index);
    return date;
  });
  const filter = h("select", { "aria-label": "Proveedor" },
    ...[["all", "Todos"], ["claude", "Claude"], ["codex", "Codex"]].map(([value, text]) => h("option", { value, text })));
  const months = h("div", { class: "usage-months" });
  for (let week = 0; week < 53; week++) {
    const monthDate = new Date(Math.min(dates[week * 7 + 6].getTime(), today.getTime()));
    months.append(h("span", { text: week === 0 || monthDate.getMonth() !== dates[week * 7 - 1].getMonth()
      ? monthDate.toLocaleDateString("es", { month: "short" }).replace(".", "") : "" }));
  }
  const grid = h("div", { class: "usage-grid", "aria-label": "Actividad de tokens por día" });
  const summary = h("span", { class: "muted" });
  const detail = h("p", { class: "usage-detail muted", role: "status", text: "Selecciona un día para ver su uso. Los días vacíos no tienen consumo registrado." });
  const card = h("div", { class: "usage-calendar" },
    h("div", { class: "usage-calendar-header" },
      h("div", {}, h("strong", { text: "Actividad de uso" }), h("p", { class: "muted", text: "Tokens por día · último año" })), filter),
    h("div", { class: "usage-calendar-scroll" },
      h("div", { class: "usage-calendar-chart" },
        h("div", { class: "usage-weekdays muted" }, ...["", "Lun", "", "Mié", "", "Vie", ""].map(text => h("span", { text }))),
        h("div", {}, months, grid))),
    h("div", { class: "usage-calendar-footer" }, summary,
      h("div", { class: "usage-legend muted" }, h("span", { text: "Menos" }),
        ...[0, 1, 2, 3, 4].map(level => h("span", { class: `usage-cell level-${level}`, "aria-hidden": "true" })), h("span", { text: "Más" }))), detail);

  function draw() {
    const values = new Map<string, { tokens: number; turns: number }>();
    for (const day of days) {
      if (filter.value !== "all" && day.provider !== filter.value || day.date < key(start) || day.date > key(today)) continue;
      const previous = values.get(day.date) ?? { tokens: 0, turns: 0 };
      values.set(day.date, { tokens: previous.tokens + day.input + day.cached + day.output, turns: previous.turns + day.turns });
    }
    const maximum = Math.max(1, ...[...values.values()].map(value => value.tokens));
    summary.textContent = `${k([...values.values()].reduce((sum, value) => sum + value.tokens, 0))} tokens registrados`;
    grid.replaceChildren(...dates.map(date => {
      const value = values.get(key(date)) ?? { tokens: 0, turns: 0 };
      const level = value.tokens === 0 ? 0 : Math.min(4, Math.max(1, Math.ceil(value.tokens / maximum * 4)));
      const description = `${date.toLocaleDateString("es", { day: "numeric", month: "long", year: "numeric" })} · ${value.tokens.toLocaleString("es")} tokens · ${value.turns} turnos`;
      const cell = h("button", { type: "button", class: `usage-cell level-${level}`, title: description, "aria-label": description });
      if (date > today) { cell.disabled = true; cell.style.visibility = "hidden"; }
      cell.addEventListener("click", () => { detail.textContent = description; });
      return cell;
    }));
    detail.textContent = "Selecciona un día para ver su uso. Los días vacíos no tienen consumo registrado.";
  }
  filter.addEventListener("change", draw);
  draw();
  return card;
}
