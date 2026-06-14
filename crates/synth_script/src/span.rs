//! Byte-offset spans and line/column resolution for diagnostics.

/// A half-open byte range `[start, end)` into the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    #[must_use]
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// The smallest span covering both `self` and `other`.
    #[must_use]
    pub fn to(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    #[must_use]
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// 1-based line and column for a byte offset, for human-readable diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

impl LineCol {
    /// Resolve a byte `offset` within `source` to a 1-based line/column.
    #[must_use]
    pub fn resolve(source: &str, offset: u32) -> Self {
        let offset = offset as usize;
        let mut line = 1u32;
        let mut col = 1u32;
        for (i, ch) in source.char_indices() {
            if i >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        Self { line, col }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_merge_and_len() {
        let a = Span::new(2, 5);
        let b = Span::new(8, 10);
        assert_eq!(a.to(b), Span::new(2, 10));
        assert_eq!(a.len(), 3);
        assert!(!a.is_empty());
        assert!(Span::new(4, 4).is_empty());
    }

    #[test]
    fn linecol_resolves_across_newlines() {
        let src = "ab\ncde\nf";
        assert_eq!(LineCol::resolve(src, 0), LineCol { line: 1, col: 1 });
        assert_eq!(LineCol::resolve(src, 1), LineCol { line: 1, col: 2 });
        assert_eq!(LineCol::resolve(src, 3), LineCol { line: 2, col: 1 });
        assert_eq!(LineCol::resolve(src, 7), LineCol { line: 3, col: 1 });
    }
}
