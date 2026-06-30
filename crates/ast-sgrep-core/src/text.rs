#[derive(Debug, Clone)]
pub struct SplitLines {
    pub lines: Vec<(u32, String)>,
    pub eol: &'static str,
}

pub fn split_content_lines(content: &str) -> SplitLines {
    if content.is_empty() {
        return SplitLines {
            lines: vec![(1, String::new())],
            eol: "lf",
        };
    }
    let eol = if content.contains("\r\n") { "crlf" } else { "lf" };
    let lines = content
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            let stripped = line.strip_suffix('\r').unwrap_or(line);
            ((i + 1) as u32, stripped.to_string())
        })
        .collect();
    SplitLines { lines, eol }
}
