//! A parse-time failure, anchored to the token span that caused it, so
//! it can become a line/column-accurate `Diagnostic`.

use crate::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

impl ParseError {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}
