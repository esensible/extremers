//! Diagnostics: a stable rule id, a span, the facts, and a concrete fix.

use serde_json::{Value as Json, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start: start as u32, end: end as u32 }
    }
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub rule: &'static str,
    pub severity: Severity,
    /// `None` for diagnostics about the interface rather than the source.
    pub span: Option<Span>,
    pub message: String,
    pub help: Option<String>,
}

impl Diag {
    pub fn error(rule: &'static str, span: Span, message: impl Into<String>) -> Diag {
        Diag { rule, severity: Severity::Error, span: Some(span), message: message.into(), help: None }
    }
    pub fn interface(rule: &'static str, message: impl Into<String>) -> Diag {
        Diag { rule, severity: Severity::Error, span: None, message: message.into(), help: None }
    }
    pub fn help(mut self, help: impl Into<String>) -> Diag {
        self.help = Some(help.into());
        self
    }

    /// `file:line:col: error[RULE]: message`, then `  help: …`.
    pub fn render(&self, file: &str, src: &str) -> String {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let at = match self.span {
            Some(s) => {
                let (l, c) = line_col(src, s.start);
                format!("{file}:{l}:{c}")
            }
            None => "interface".to_string(),
        };
        let mut out = format!("{at}: {sev}[{}]: {}", self.rule, self.message);
        if let Some(h) = &self.help {
            out.push_str("\n  help: ");
            out.push_str(h);
        }
        out
    }

    /// The JSON form (spec §5). Each end of the span has its line and column
    /// and its byte offset in `src` (for editors that address by offset).
    pub fn to_json(&self, src: &str) -> Json {
        let span = self.span.map(|s| {
            let (l1, c1) = line_col(src, s.start);
            let (l2, c2) = line_col(src, s.end);
            json!({"start": {"line": l1, "col": c1, "offset": s.start}, "end": {"line": l2, "col": c2, "offset": s.end}})
        });
        json!({
            "rule": self.rule,
            "severity": match self.severity { Severity::Error => "error", Severity::Warning => "warning" },
            "span": span,
            "message": self.message,
            "help": self.help,
        })
    }
}

/// What to do about a run-time error (spec §5.2).
pub fn runtime_help(code: &str) -> &'static str {
    match code {
        "R-NIL" => "a nullable input or field is nil here: test it first (`x != nil ? x.f : …`), or read it with `?.` and give a default with `??`",
        "R-INDEX" => "the index is out of range: check `len(xs)` first, or use `get(xs, i)`, which gives nil",
        "R-DIV0" => "`%` by zero: check the divisor first",
        "R-ARG" => "a builtin was given something it can't use (e.g. `reduce` with no initial value on an empty list, or a negative range)",
        "R-INPUT" => "an input doesn't match its schema: this is the host's or the sample's mistake, not the program's",
        _ => "this means a bug in the compiler: please report it with the program and the inputs",
    }
}

/// The source text of `span`, on one line, shortened to about 40 characters.
pub fn snippet(src: &str, span: Span) -> String {
    let (a, b) = ((span.start as usize).min(src.len()), (span.end as usize).min(src.len()));
    let text: String = src.get(a..b).unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 40 { format!("{}…", text.chars().take(39).collect::<String>()) } else { text }
}

/// 1-based line and column (in characters) of a byte offset.
pub fn line_col(src: &str, offset: u32) -> (usize, usize) {
    let offset = (offset as usize).min(src.len());
    let before = &src[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}
