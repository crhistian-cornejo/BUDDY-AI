//! Local iCalendar previews. Supports IANA time zones, all-day events, recurrence rules and single-instance exceptions.
use std::{collections::HashSet, io::{BufReader, Read}, path::Path};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use ical::{IcalParser, property::Property, parser::ical::component::IcalEvent};
use rrule::{RRuleSet, Tz};
use serde::Serialize;
use crate::CoreError;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Appointment {
    pub title: String, pub start: i64, pub end: i64, pub all_day: bool,
    pub location: String, pub source: String, pub url: Option<String>,
}

pub fn now() -> i64 { Utc::now().timestamp() }
fn error(message: impl Into<String>) -> CoreError { CoreError::Io(message.into()) }
fn prop<'a>(event: &'a IcalEvent, name: &str) -> Option<&'a Property> {
    event.properties.iter().find(|p| p.name == name)
}
fn value(event: &IcalEvent, name: &str) -> String {
    prop(event, name).and_then(|p| p.value.clone()).unwrap_or_default()
}
fn param(property: &Property, name: &str) -> Option<String> {
    property.params.as_ref()?.iter().find(|(key, _)| key == name)?.1.first().cloned()
}
fn unescape(text: &str) -> String {
    let mut out = String::new(); let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' { if let Some(c) = chars.next() { out.push(if c == 'n' || c == 'N' { '\n' } else { c }); } }
        else { out.push(c); }
    }
    out
}
fn zone(name: &str) -> Result<Tz, CoreError> {
    let name = name.trim_matches('"');
    let name = name.split("/Tzfile/").last().unwrap_or(name);
    name.parse::<chrono_tz::Tz>().map(Tz::Tz).map_err(|_| error(format!("La zona horaria «{name}» de esta agenda no se reconoce. Exporta la agenda usando una zona IANA.")))
}
fn date(property: &Property, fallback: Tz) -> Result<(DateTime<Tz>, bool), CoreError> {
    let raw = property.value.as_deref().ok_or_else(|| error("Falta una fecha de la agenda."))?;
    let all_day = param(property, "VALUE").as_deref() == Some("DATE") || raw.len() == 8;
    // DATE is a civil day, independent of the source calendar's time zone.
    let tz = if all_day { Tz::Local(chrono::Local) } else if raw.ends_with('Z') { Tz::UTC } else { param(property, "TZID").map(|v| zone(&v)).transpose()?.unwrap_or(fallback) };
    let naive = if all_day {
        NaiveDate::parse_from_str(raw, "%Y%m%d").map_err(|_| error("Fecha de agenda inválida."))?.and_hms_opt(0, 0, 0).unwrap()
    } else { NaiveDateTime::parse_from_str(raw.trim_end_matches('Z'), "%Y%m%dT%H%M%S").map_err(|_| error("Hora de agenda inválida."))? };
    let dt = tz.from_local_datetime(&naive).earliest().ok_or_else(|| error("La hora de un evento no existe en su zona horaria."))?;
    Ok((dt, all_day))
}
fn duration(raw: &str) -> Option<i64> {
    let mut raw = raw.strip_prefix('P')?;
    let mut total = 0i64; let mut number = String::new();
    if let Some(s) = raw.strip_prefix('+') { raw = s; }
    for c in raw.chars() {
        if c.is_ascii_digit() { number.push(c); continue; }
        if c == 'T' && number.is_empty() { continue; }
        let n = number.parse::<i64>().ok()?; number.clear();
        total = total.checked_add(n.checked_mul(match c { 'W' => 604800, 'D' => 86400, 'H' => 3600, 'M' => 60, 'S' => 1, _ => return None })?)?;
    }
    if number.is_empty() { Some(total) } else { None }
}

pub fn read(path: &Path, at: i64) -> Result<Option<Appointment>, CoreError> {
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ics")) { return Err(error("Selecciona una agenda .ics.")); }
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > 5_000_000 { return Err(error("La agenda debe ocupar menos de 5 MB.")); }
    let mut content = String::new(); file.take(5_000_001).read_to_string(&mut content)?;
    if content.len() > 5_000_000 { return Err(error("La agenda debe ocupar menos de 5 MB.")); }
    next(&content, path.file_name().unwrap_or_default().to_string_lossy().as_ref(), at)
}

pub fn next(content: &str, source: &str, at: i64) -> Result<Option<Appointment>, CoreError> {
    let mut next: Option<Appointment> = None; let mut count = 0;
    for calendar in IcalParser::new(BufReader::new(content.as_bytes())) {
        let calendar = calendar.map_err(|_| error("No se pudo interpretar esta agenda .ics."))?; count += 1;
        let default_zone = calendar.properties.iter().find(|p| p.name == "X-WR-TIMEZONE")
            .and_then(|p| p.value.as_deref()).map(zone).transpose()?.unwrap_or(Tz::Local(chrono::Local));
        let label = calendar.properties.iter().find(|p| p.name == "X-WR-CALNAME")
            .and_then(|p| p.value.as_deref()).map(unescape).unwrap_or_else(|| source.into());
        let mut overrides = HashSet::new();
        for event in &calendar.events {
            if let Some(recurrence) = prop(event, "RECURRENCE-ID") {
                if param(recurrence, "RANGE").is_some() { return Err(error("Esta agenda contiene cambios de serie no compatibles. Exporta sus eventos como instancias.")); }
                overrides.insert((value(event, "UID"), date(recurrence, default_zone)?.0.timestamp()));
            }
        }
        for event in &calendar.events {
            if value(event, "STATUS") == "CANCELLED" { continue; }
            let Some(start_prop) = prop(event, "DTSTART") else { continue; };
            let (start, all_day) = date(start_prop, default_zone)?;
            let duration = if let Some(end) = prop(event, "DTEND") { date(end, default_zone)?.0.timestamp() - start.timestamp() }
                else if let Some(d) = prop(event, "DURATION") { duration(d.value.as_deref().unwrap_or("")).ok_or_else(|| error("Duración de evento inválida."))? }
                else if all_day { 86400 } else { 0 };
            let duration = duration.max(0);
            let day_span = if all_day {
                if let Some(end) = prop(event, "DTEND") {
                    date(end, default_zone)?.0.date_naive().signed_duration_since(start.date_naive()).num_days().max(1) as u64
                } else { ((duration / 86400).max(1)) as u64 }
            } else { 0 };
            let recurring = event.properties.iter().any(|p| p.name == "RRULE" || p.name == "RDATE");
            let dates = if recurring {
                let mut set = RRuleSet::new(start);
                for property in event.properties.iter().filter(|p| matches!(p.name.as_str(), "RRULE" | "EXRULE")) {
                    let raw = format!("{}:{}", property.name, property.value.as_deref().unwrap_or(""));
                    set = set.set_from_string(&raw).map_err(|_| error("No se pudo interpretar la repetición de un evento."))?;
                }
                for property in event.properties.iter().filter(|p| matches!(p.name.as_str(), "RDATE" | "EXDATE")) {
                    for raw in property.value.as_deref().unwrap_or("").split(',') {
                        let mut p = property.clone(); p.value = Some(raw.into()); let d = date(&p, default_zone)?.0;
                        set = if property.name == "RDATE" { set.rdate(d) } else { set.exdate(d) };
                    }
                }
                let after = Tz::UTC.timestamp_opt(at.saturating_sub(duration).saturating_sub(1), 0).single().ok_or_else(|| error("Fecha fuera de rango."))?;
                let before = Tz::UTC.timestamp_opt(at.saturating_add(366 * 86400), 0).single().ok_or_else(|| error("Fecha fuera de rango."))?;
                let result = set.after(after).before(before).all(512);
                result.dates
            } else { vec![start] };
            for occurrence in dates {
                let timestamp = occurrence.timestamp();
                let end = if all_day {
                    occurrence.checked_add_days(chrono::Days::new(day_span)).ok_or_else(|| error("Fecha fuera de rango."))?.timestamp()
                } else { timestamp.saturating_add(duration) };
                if end <= at && timestamp < at { continue; }
                if prop(event, "RECURRENCE-ID").is_none() && overrides.contains(&(value(event, "UID"), timestamp)) { continue; }
                let url = value(event, "URL");
                let appointment = Appointment { title: unescape(&value(event, "SUMMARY")), start: timestamp, end, all_day,
                    location: unescape(&value(event, "LOCATION")), source: label.clone(),
                    url: if url.starts_with("https://") || url.starts_with("http://") { Some(url) } else { None } };
                if next.as_ref().is_none_or(|n| appointment.start < n.start) { next = Some(appointment); }
                break;
            }
        }
    }
    if count == 0 { return Err(error("Este archivo no contiene una agenda .ics.")); }
    Ok(next)
}

fn escape(text: &str) -> String { text.replace('\\', "\\\\").replace('\r', "").replace('\n', "\\n").replace(';', "\\;").replace(',', "\\,") }
pub fn export(event: &Appointment, folder: &Path) -> Result<String, CoreError> {
    std::fs::create_dir_all(folder)?;
    let format_date = |time: i64| -> Result<String, CoreError> {
        let date = Utc.timestamp_opt(time, 0).single().ok_or_else(|| error("Fecha fuera de rango."))?;
        Ok(date.format("%Y%m%dT%H%M%SZ").to_string())
    };
    let date = if event.all_day {
        let civil_date = |time| DateTime::from_timestamp(time, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y%m%d").to_string()).ok_or_else(|| error("Fecha fuera de rango."));
        format!("DTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\n", civil_date(event.start)?, civil_date(event.end)?)
    } else { format!("DTSTART:{}\r\nDTEND:{}\r\n", format_date(event.start)?, format_date(event.end)?) };
    let ics = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Buddy//Notch//ES\r\nBEGIN:VEVENT\r\nUID:buddy-{}\r\nDTSTAMP:{}\r\n{date}SUMMARY:{}\r\nLOCATION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n", event.start, format_date(now())?, escape(&event.title), escape(&event.location));
    let path = folder.join("proxima-cita.ics"); std::fs::write(&path, ics)?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wrap(event: &str) -> String { format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{event}END:VCALENDAR\r\n") }
    #[test]
    fn ignores_past_and_cancelled_events_and_keeps_real_source() {
        let text = wrap("X-WR-CALNAME:Trabajo\r\nBEGIN:VEVENT\r\nDTSTART:20261003T150000Z\r\nSUMMARY:Revisión\\, equipo\r\nLOCATION:Sala 1\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20261003T130000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n");
        let at = Utc.with_ymd_and_hms(2026,10,3,14,0,0).unwrap().timestamp();
        let next = next(&text, "test.ics", at).unwrap().unwrap();
        assert_eq!(next.source, "Trabajo"); assert_eq!(next.title, "Revisión, equipo");
        assert!(super::next(&text, "test.ics", at + 86400).unwrap().is_none());
    }
    #[test]
    fn recurring_event_honors_exdates_timezones_and_overrides() {
        let text = wrap("BEGIN:VEVENT\r\nUID:a\r\nDTSTART;TZID=America/Lima:20261001T100000\r\nRRULE:FREQ=DAILY;COUNT=10\r\nEXDATE;TZID=America/Lima:20261003T100000\r\nSUMMARY:Daily\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:a\r\nRECURRENCE-ID;TZID=America/Lima:20261004T100000\r\nDTSTART;TZID=America/Lima:20261004T120000\r\nSUMMARY:Moved\r\nEND:VEVENT\r\n");
        let at = Utc.with_ymd_and_hms(2026,10,3,14,0,0).unwrap().timestamp();
        let next = next(&text, "test.ics", at).unwrap().unwrap();
        assert_eq!(next.title, "Moved");
        assert_eq!(next.start, Utc.with_ymd_and_hms(2026,10,4,17,0,0).unwrap().timestamp());
    }
    #[test]
    fn recognizes_all_day_and_ongoing_events() {
        let text = wrap("X-WR-TIMEZONE:America/Lima\r\nBEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261003\r\nDTEND;VALUE=DATE:20261004\r\nSUMMARY:Día libre\r\nEND:VEVENT\r\n");
        let at = Utc.with_ymd_and_hms(2026,10,3,14,0,0).unwrap().timestamp();
        assert!(next(&text, "test.ics", at).unwrap().unwrap().all_day);
        assert!(next("invalid", "test.ics", at).is_err());
    }
    #[test]
    fn exporting_an_instance_preserves_dates_duration_and_escaped_text() {
        let at = Utc.with_ymd_and_hms(2026,10,3,0,0,0).unwrap().timestamp();
        let content = wrap("X-WR-TIMEZONE:Pacific/Auckland\r\nBEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261004\r\nDTEND;VALUE=DATE:20261007\r\nSUMMARY:Notas\\, revisión\\nEquipo\r\nLOCATION:Sala\\; A\r\nEND:VEVENT\r\n");
        let appointment = next(&content, "test.ics", at).unwrap().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let output = export(&appointment, folder.path()).unwrap();
        let text = std::fs::read_to_string(output).unwrap();
        assert!(text.contains("DTSTART;VALUE=DATE:20261004\r\nDTEND;VALUE=DATE:20261007"));
        let restored = next(&text, "test.ics", at).unwrap().unwrap();
        assert_eq!(restored.start, appointment.start);
        assert_eq!(restored.end, appointment.end);
        assert_eq!(restored.title, appointment.title);
        assert_eq!(restored.location, appointment.location);
    }
}
