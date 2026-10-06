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
}

impl Diagnostics {
    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push(Diagnostic {
            span,
            message: message.into(),
        });
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn render(&self, files: &[SourceFile]) -> String {
        let mut out = String::new();
        for err in &self.errors {
            let file = files
                .iter()
                .find(|f| f.id == err.span.file)
                .or_else(|| files.first());
            if let Some(file) = file {
                let (line, col) = file.loc(err.span.start as usize);
                fmt::write(
                    &mut out,
                    format_args!("{}:{}:{}: error: {}\n", file.path, line, col, err.message),
                )
                .ok();
            } else {
                fmt::write(&mut out, format_args!("error: {}\n", err.message)).ok();
            }
        }
        out
    }
}
