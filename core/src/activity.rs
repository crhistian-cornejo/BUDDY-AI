//! What Buddy is doing right now, in one word the apps can draw: the tool an agent uses becomes a kind (Word,
//! Excel, PowerPoint, the web…) and a short Spanish label. The notch / top bar animate the kind's icon while the
//! turn runs; the chat keeps its own activity line.

/// The kind of work and the label shown with it.
pub fn of_tool(name: &str) -> (&'static str, &'static str) {
    // A connector (Context7, Microsoft Learn, DeepWiki…): documentation over the network.
    if crate::connectors::owns_tool(name) {
        return ("web", "Consultando documentación");
    }
    // The user's claude.ai accounts.
    match crate::accounts::service_of(name) {
        Some("gmail") => return ("read", "Revisando tu correo"),
        Some("drive") => return ("read", "Leyendo tu Drive"),
        Some(_) => return ("edit", "Anotando en Notion"),
        None => {}
    }
    let tool = name.rsplit("__").next().unwrap_or(name);
    match tool {
        "create_document" => ("word", "Escribiendo un Word"),
        "create_spreadsheet" => ("excel", "Armando un Excel"),
        "create_presentation" => ("powerpoint", "Preparando diapositivas"),
        "read_document" | "Read" | "Glob" | "Grep" | "LS" => ("read", "Leyendo archivos"),
        "WebSearch" | "web_search" | "webSearch" => ("web", "Buscando en la web"),
        "WebFetch" => ("web", "Leyendo una página"),
        "Bash" | "shell" | "commandExecution" | "exec_command" => ("command", "Ejecutando un comando"),
        "Edit" | "Write" | "MultiEdit" | "apply_patch" | "fileChange" => ("edit", "Editando archivos"),
        "look_at_screen" => ("screen", "Mirando tu pantalla"),
        "media_control" | "media_play" | "media_search" | "now_playing" | "spotify_search" | "spotify_playlist" => ("music", "Con la música"),
        "use_skill" => ("skill", "Usando una habilidad"),
        "generate_image" | "image_gen" | "imagegen" => ("image", "Creando la imagen"),
        _ => ("tool", "Usando una herramienta"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_become_kinds() {
        assert_eq!(of_tool("mcp__buddy__create_document").0, "word");
        assert_eq!(of_tool("mcp__buddy__create_spreadsheet").0, "excel");
        assert_eq!(of_tool("mcp__buddy__create_presentation").0, "powerpoint");
        assert_eq!(of_tool("WebSearch"), ("web", "Buscando en la web"));
        assert_eq!(of_tool("Bash").0, "command");
        assert_eq!(of_tool("mcp__buddy__look_at_screen").0, "screen");
        assert_eq!(of_tool("mcp__buddy__spotify_search").0, "music");
        assert_eq!(of_tool("algo_raro").0, "tool");
        assert_eq!(of_tool("mcp__context7__query-docs"), ("web", "Consultando documentación"));
        assert_eq!(of_tool("mcp__claude_ai_Gmail__search_threads"), ("read", "Revisando tu correo"));
        assert_eq!(of_tool("mcp__claude_ai_Notion__notion-create-pages"), ("edit", "Anotando en Notion"));
    }
}
