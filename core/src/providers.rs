//! Providers: Claude (Claude Code CLI, stream-json), Codex (`codex app-server`) and Gemini (Antigravity CLI,
//! `agy -p --output-format stream-json`) behind one `Provider` trait: status, streaming turns, cancel.
//! Ported from MIKA in phase 1 (Claude, Codex) and phase 3 (Antigravity). See docs/REUSE-FROM-MIKA.md.
