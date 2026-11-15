use crate::span::{SourceFile, Span};
use std::fmt;

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Default, Debug)]
pub struct Diagnostics {
    pub errors: Vec<Diagnostic>,
    /// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the W-*
    /// warnings (the T5b shape detector is the first consumer). A
    /// warning NEVER blocks the build — `is_empty` stays errors-only;
    /// the driver renders them to stderr after a successful check.
    pub warnings: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push(Diagnostic {
            span,
            message: message.into(),
        });
    }

    /// Wave 14 (Impl-S): a named, spanned advisory — the program is
    /// legal but a measurable property is being wasted (T5b: the
    /// lexicographic double loop over SFC storage).
    pub fn warn(&mut self, span: Span, message: impl Into<String>) {
        self.warnings.push(Diagnostic {
            span,
            message: message.into(),
        });
    }

    /// Errors only — warnings never gate the build.
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    fn render_kind(&self, files: &[SourceFile], diags: &[Diagnostic], kind: &str) -> String {
        let mut out = String::new();
        for err in diags {
            let file = files
                .iter()
                .find(|f| f.id == err.span.file)
                .or_else(|| files.first());
            if let Some(file) = file {
                let (line, col) = file.loc(err.span.start as usize);
                fmt::write(
                    &mut out,
                    format_args!("{}:{}:{}: {}: {}\n", file.path, line, col, kind, err.message),
                )
                .ok();
            } else {
                fmt::write(&mut out, format_args!("{}: {}\n", kind, err.message)).ok();
            }
        }
        out
    }

    pub fn render(&self, files: &[SourceFile]) -> String {
        self.render_kind(files, &self.errors, "error")
    }

    /// Wave 14 (Impl-S): the warnings' stderr render (same shape as
    /// the errors', the kind string aside).
    pub fn render_warnings(&self, files: &[SourceFile]) -> String {
        self.render_kind(files, &self.warnings, "warning")
    }
}
