//! Voice. The Mac dictates into the composer with the system's Speech framework (apps/macos/Sources/Voice); Windows
//! will transcribe with whisper.cpp here. No audio leaves the machine. A wake phrase («Hey Buddy») was tried and
//! dropped: without the system's low-power trigger it keeps the transcriber working on any sound in the room.
//!
//! What both platforms share lives here: the words a recogniser should expect, and the Spanish touches a recogniser
//! leaves out (the opening «¿» and «¡», a capital after a full stop).

/// Names a recogniser would otherwise spell by ear («Badi», «Nico», «noshon»).
pub const VOCABULARY: [&str; 22] = [
    "Buddy", "Niko", "Parley", "Notion", "Gmail", "Yape", "Plin", "BCP", "Interbank", "BBVA", "Scotiabank", "Claude", "Codex", "Gemini", "ChatGPT",
    "Telegram", "Spotify", "Excel", "PowerPoint", "PDF", "dashboard", "presupuesto",
];

/// Dictated text as Spanish writes it: «¿» and «¡» open what «?» and «!» close, and a sentence starts with a capital.
pub fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    // Where the sentence being written starts in `out`, and whether it already has its opening mark.
    let mut start = 0;
    let (mut asked, mut exclaimed) = (false, false);
    let mut capital = true;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '¿' => asked = true,
            '¡' => exclaimed = true,
            _ => {}
        }
        if capital && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            capital = false;
            continue;
        }
        if capital && c.is_numeric() {
            capital = false;
        }
        out.push(c);
        if matches!(c, '?' | '!') {
            let (opened, mark) = if c == '?' { (asked, '¿') } else { (exclaimed, '¡') };
            if !opened {
                let at = start + out[start..].len() - out[start..].trim_start().len();
                out.insert(at, mark);
            }
        }
        // A full stop ends a sentence only before a space («informe.pdf» and «3.50» go on).
        let ends = matches!(c, '?' | '!' | '…' | '\n') || (c == '.' && chars.peek().is_none_or(|next| next.is_whitespace()));
        if ends {
            start = out.len();
            (asked, exclaimed, capital) = (false, false, true);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictation_reads_like_written_spanish() {
        assert_eq!(tidy("hola buddy. cuánto voy gastando este mes? qué bueno!"), "Hola buddy. ¿Cuánto voy gastando este mes? ¡Qué bueno!");
        assert_eq!(tidy("¿Ya está? sí, gracias"), "¿Ya está? Sí, gracias");
        assert_eq!(tidy("son 3.50 soles"), "Son 3.50 soles");
        assert_eq!(tidy(""), "");
        assert_eq!(tidy("abre el archivo informe.pdf y dime"), "Abre el archivo informe.pdf y dime");
    }
}
