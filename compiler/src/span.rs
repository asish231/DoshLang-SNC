use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: u32,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: u32, start: usize, end: usize) -> Self {
        Self {
            file,
            start: start as u32,
            end: end as u32,
        }
    }

    pub fn dummy() -> Self {
        Self {
            file: 0,
            start: 0,
            end: 0,
        }
    }

    pub fn merge(self, other: Span) -> Span {
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    pub id: u32,
    pub path: String,
    pub src: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(id: u32, path: String, src: String) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            id,
            path,
            src,
            line_starts,
        }
    }

    pub fn loc(&self, offset: usize) -> (u32, u32) {
        let mut lo = 0;
        let mut hi = self.line_starts.len();
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if self.line_starts[mid] <= offset {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let line = lo as u32 + 1;
        let col = (offset.saturating_sub(self.line_starts[lo]) + 1) as u32;
        (line, col)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}
