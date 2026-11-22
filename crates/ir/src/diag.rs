use crate::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Note,
}

/// A compiler diagnostic. Codes:
/// - `P0xx` frontend / protocol
/// - `P1xx` unsupported syntax or construct
/// - `P2xx` dynamic behaviour requiring the CPython bridge
/// - `P3xx` inference failures
#[derive(Clone, Debug, serde::Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub span: Span,
    pub message: String,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Error, code, span, message: message.into(), notes: vec![] }
    }

    pub fn warning(code: &'static str, span: Span, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, code, span, message: message.into(), notes: vec![] }
    }

    pub fn note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Render with `files[span.unit]` as the file name.
    pub fn render_in(&self, files: &[String]) -> String {
        let f = files.get(self.span.unit as usize).or(files.first()).map(|s| s.as_str()).unwrap_or("?");
        self.render(f)
    }

    pub fn render(&self, file: &str) -> String {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        };
        let mut s = format!("{file}:{}  {}  {sev}: {}", self.span, self.code, self.message);
        for n in &self.notes {
            s.push_str("\n    ");
            s.push_str(n);
        }
        s
    }
}
