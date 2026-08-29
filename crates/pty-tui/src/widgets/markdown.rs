//! Markdown renderer (`src/tui/widgets/markdown.ts`): a CommonMark subset —
//! headings 1-4, paragraphs (optionally wrapped), bold / italic / inline
//! code / links, fenced code, bullet and ordered lists, task lists,
//! blockquotes and horizontal rules — parsed into blocks and rendered as
//! lines with a blank row between blocks and a separator for a rule.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::{Color, Theme};

/// A block (`Block`, `markdown.ts:28-45`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading { level: usize, text: String },
    Paragraph(String),
    Code(Vec<String>),
    Bullet(Vec<String>),
    Ordered(Vec<String>),
    Task(Vec<(bool, String)>),
    Quote(String),
    Hr,
}

fn heading(line: &str) -> Option<(usize, String)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 4 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    Some((hashes, rest.trim_start().to_string()))
}

fn list_item(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('-').or_else(|| line.strip_prefix('*'))?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    Some(rest.trim_start())
}

fn task_item(line: &str) -> Option<(bool, String)> {
    let rest = list_item(line)?;
    let inner = rest.strip_prefix('[')?;
    let mut chars = inner.chars();
    let mark = chars.next()?;
    if !matches!(mark, ' ' | 'x' | 'X') || chars.next()? != ']' {
        return None;
    }
    let after = chars.as_str();
    if !after.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    Some((mark != ' ', after.trim_start().to_string()))
}

fn ordered_item(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let rest = line[digits..].strip_prefix('.')?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    Some(rest.trim_start())
}

fn quote_line(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn is_hr(line: &str) -> bool {
    let t = line.trim_end();
    for ch in ['-', '*', '_'] {
        if t.len() >= 3 && t.chars().all(|c| c == ch) {
            return true;
        }
    }
    false
}

/// `parseMarkdown` (`markdown.ts:56-158`).
pub fn parse_markdown(source: &str) -> Vec<Block> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let flush = |para: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph(para.join(" ")));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("```") {
            flush(&mut para, &mut blocks);
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].starts_with("```") {
                code.push(lines[i].to_string());
                i += 1;
            }
            blocks.push(Block::Code(code));
            i += 1;
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut para, &mut blocks);
            i += 1;
            continue;
        }
        if is_hr(line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Hr);
            i += 1;
            continue;
        }
        if let Some((level, text)) = heading(line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading { level, text });
            i += 1;
            continue;
        }
        if let Some(task) = task_item(line) {
            flush(&mut para, &mut blocks);
            match blocks.last_mut() {
                Some(Block::Task(tasks)) => tasks.push(task),
                _ => blocks.push(Block::Task(vec![task])),
            }
            i += 1;
            continue;
        }
        if let Some(item) = list_item(line) {
            flush(&mut para, &mut blocks);
            match blocks.last_mut() {
                Some(Block::Bullet(items)) => items.push(item.to_string()),
                _ => blocks.push(Block::Bullet(vec![item.to_string()])),
            }
            i += 1;
            continue;
        }
        if let Some(item) = ordered_item(line) {
            flush(&mut para, &mut blocks);
            match blocks.last_mut() {
                Some(Block::Ordered(items)) => items.push(item.to_string()),
                _ => blocks.push(Block::Ordered(vec![item.to_string()])),
            }
            i += 1;
            continue;
        }
        if let Some(q) = quote_line(line) {
            flush(&mut para, &mut blocks);
            match blocks.last_mut() {
                Some(Block::Quote(text)) => {
                    text.push('\n');
                    text.push_str(q);
                }
                _ => blocks.push(Block::Quote(q.to_string())),
            }
            i += 1;
            continue;
        }
        para.push(line.to_string());
        i += 1;
    }
    flush(&mut para, &mut blocks);
    blocks
}

/// An inline segment (`InlineSegment`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InlineSegment {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub url: Option<String>,
}

impl InlineSegment {
    pub fn plain(text: &str) -> Self {
        InlineSegment {
            text: text.to_string(),
            ..Default::default()
        }
    }
}

/// `parseInline` (`markdown.ts:172-237`).
pub fn parse_inline(src: &str) -> Vec<InlineSegment> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..chars.len()).find(|&j| chars[j..].starts_with(pat))
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '['
            && let Some(end) = find(i + 1, &[']'])
            && chars.get(end + 1) == Some(&'(')
            && let Some(url_end) = find(end + 2, &[')'])
        {
            if !buf.is_empty() {
                out.push(InlineSegment::plain(&std::mem::take(&mut buf)));
            }
            out.push(InlineSegment {
                text: chars[i + 1..end].iter().collect(),
                url: Some(chars[end + 2..url_end].iter().collect()),
                ..Default::default()
            });
            i = url_end + 1;
            continue;
        }
        if c == '`'
            && let Some(end) = find(i + 1, &['`'])
        {
            if !buf.is_empty() {
                out.push(InlineSegment::plain(&std::mem::take(&mut buf)));
            }
            out.push(InlineSegment {
                text: chars[i + 1..end].iter().collect(),
                code: true,
                ..Default::default()
            });
            i = end + 1;
            continue;
        }
        if c == '*' {
            if chars.get(i + 1) == Some(&'*') {
                if let Some(end) = find(i + 2, &['*', '*']) {
                    if !buf.is_empty() {
                        out.push(InlineSegment::plain(&std::mem::take(&mut buf)));
                    }
                    out.push(InlineSegment {
                        text: chars[i + 2..end].iter().collect(),
                        bold: true,
                        ..Default::default()
                    });
                    i = end + 2;
                    continue;
                }
                buf.push_str("**");
                i += 2;
                continue;
            }
            if let Some(end) = find(i + 1, &['*']) {
                if !buf.is_empty() {
                    out.push(InlineSegment::plain(&std::mem::take(&mut buf)));
                }
                out.push(InlineSegment {
                    text: chars[i + 1..end].iter().collect(),
                    italic: true,
                    ..Default::default()
                });
                i = end + 1;
                continue;
            }
            buf.push('*');
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    if !buf.is_empty() {
        out.push(InlineSegment::plain(&buf));
    }
    out
}

fn render_inline(theme: &Theme, segments: &[InlineSegment], base: Color) -> Vec<Span<'static>> {
    segments
        .iter()
        .map(|s| {
            if s.code {
                return Span::styled(s.text.clone(), Style::default().fg(theme.color(Color::Accent)));
            }
            if s.url.is_some() {
                return Span::styled(format!("{} ", s.text), Style::default().fg(theme.color(Color::Accent)));
            }
            let mut style = Style::default().fg(theme.color(base));
            if s.bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            if s.italic {
                style = style.add_modifier(Modifier::ITALIC);
            }
            Span::styled(s.text.clone(), style)
        })
        .collect()
}

fn heading_color(level: usize) -> Color {
    match level {
        1 | 3 => Color::Accent,
        2 => Color::Primary,
        _ => Color::Muted,
    }
}

/// `wrapLine`: word wrap keeping whole words; `None` = no wrapping.
pub fn wrap_line(src: &str, width: Option<usize>) -> Vec<String> {
    let Some(width) = width.filter(|w| *w > 0) else {
        return vec![src.to_string()];
    };
    // Split keeping whitespace runs as their own tokens (`split(/(\s+)/)`).
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_ws: Option<bool> = None;
    for c in src.chars() {
        let ws = c.is_whitespace();
        if in_ws.is_some_and(|w| w != ws) {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(c);
        in_ws = Some(ws);
    }
    words.push(cur);
    let mut lines = Vec::new();
    let mut line = String::new();
    for w in words {
        if line.chars().count() + w.chars().count() <= width {
            line.push_str(&w);
            continue;
        }
        if !line.is_empty() {
            lines.push(line.trim_end().to_string());
            line = String::new();
        }
        if w.chars().count() > width {
            let chars: Vec<char> = w.chars().collect();
            for chunk in chars.chunks(width) {
                lines.push(chunk.iter().collect());
            }
        } else if !w.trim().is_empty() {
            line = w;
        }
    }
    if !line.is_empty() {
        lines.push(line.trim_end().to_string());
    }
    lines
}

/// One rendered row.
#[derive(Debug, Clone, PartialEq)]
pub enum MdRow {
    Line(Line<'static>),
    /// A blank spacer row between blocks.
    Blank,
    /// A horizontal rule (a panel separator).
    Separator,
}

/// `renderMarkdown` (`markdown.ts:277-345`).
pub fn render_markdown(theme: &Theme, source: &str, width: Option<usize>) -> Vec<MdRow> {
    let muted = Style::default().fg(theme.color(Color::Muted));
    let dim = muted.add_modifier(Modifier::DIM);
    let mut out = Vec::new();
    for (bi, b) in parse_markdown(source).iter().enumerate() {
        if bi > 0 {
            out.push(MdRow::Blank);
        }
        match b {
            Block::Heading { level, text } => {
                let c = heading_color(*level);
                let mut spans = vec![Span::styled(format!("{} ", "#".repeat(*level)), dim)];
                for (i, mut s) in render_inline(theme, &parse_inline(text), c).into_iter().enumerate() {
                    if i == 0 {
                        s.style = Style::default().fg(theme.color(c)).add_modifier(Modifier::BOLD);
                    }
                    spans.push(s);
                }
                out.push(MdRow::Line(Line::from(spans)));
            }
            Block::Paragraph(text) => {
                for ln in wrap_line(text, width) {
                    out.push(MdRow::Line(Line::from(render_inline(
                        theme,
                        &parse_inline(&ln),
                        Color::Primary,
                    ))));
                }
            }
            Block::Code(lines) => {
                for ln in lines {
                    out.push(MdRow::Line(Line::from(Span::styled(format!("  {ln}"), muted))));
                }
            }
            Block::Bullet(items) => {
                for item in items {
                    let mut spans = vec![Span::styled("  \u{2022} ", muted)];
                    spans.extend(render_inline(theme, &parse_inline(item), Color::Primary));
                    out.push(MdRow::Line(Line::from(spans)));
                }
            }
            Block::Ordered(items) => {
                for (i, item) in items.iter().enumerate() {
                    let mut spans = vec![Span::styled(format!("  {}. ", i + 1), muted)];
                    spans.extend(render_inline(theme, &parse_inline(item), Color::Primary));
                    out.push(MdRow::Line(Line::from(spans)));
                }
            }
            Block::Task(tasks) => {
                for (done, text) in tasks {
                    let box_color = if *done { Color::Muted } else { Color::Accent };
                    let mut spans = vec![
                        Span::styled("  ", muted),
                        Span::styled(
                            if *done { "\u{2611}" } else { "\u{2610}" },
                            Style::default().fg(theme.color(box_color)),
                        ),
                        Span::styled(" ", muted),
                    ];
                    spans.extend(render_inline(
                        theme,
                        &parse_inline(text),
                        if *done { Color::Muted } else { Color::Primary },
                    ));
                    out.push(MdRow::Line(Line::from(spans)));
                }
            }
            Block::Quote(text) => {
                for ln in text.split('\n') {
                    for w in wrap_line(ln, width.map(|w| w.saturating_sub(2))) {
                        let mut spans = vec![Span::styled("\u{2502} ", muted)];
                        spans.extend(render_inline(theme, &parse_inline(&w), Color::Muted));
                        out.push(MdRow::Line(Line::from(spans)));
                    }
                }
            }
            Block::Hr => out.push(MdRow::Separator),
        }
    }
    out
}

/// The rows as plain lines (blank rows empty, separators a rule of `width`).
pub fn markdown_lines(theme: &Theme, source: &str, width: Option<usize>) -> Vec<Line<'static>> {
    render_markdown(theme, source, width)
        .into_iter()
        .map(|r| match r {
            MdRow::Line(l) => l,
            MdRow::Blank => Line::raw(""),
            MdRow::Separator => Line::styled(
                "\u{2500}".repeat(width.unwrap_or(20)),
                Style::default().fg(theme.color(Color::Border)),
            ),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node: tests/widgets-markdown.test.ts:4-65
    #[test]
    fn blocks() {
        let b = parse_markdown("first para\nmore of it\n\nsecond para");
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], Block::Paragraph("first para more of it".into()));
        assert!(matches!(b[1], Block::Paragraph(_)));
        let b = parse_markdown("# h1\n## h2\n### h3\n#### h4");
        assert_eq!(b.len(), 4);
        assert_eq!(b[0], Block::Heading { level: 1, text: "h1".into() });
        assert_eq!(b[3], Block::Heading { level: 4, text: "h4".into() });
        let b = parse_markdown("```\nconst x = 1;\nconsole.log(x);\n```");
        assert_eq!(b, vec![Block::Code(vec!["const x = 1;".into(), "console.log(x);".into()])]);
        assert_eq!(parse_markdown("- a\n- b\n- c"), vec![Block::Bullet(vec!["a".into(), "b".into(), "c".into()])]);
        assert_eq!(
            parse_markdown("- [ ] todo\n- [x] done\n- [X] also done"),
            vec![Block::Task(vec![
                (false, "todo".into()),
                (true, "done".into()),
                (true, "also done".into())
            ])]
        );
        assert_eq!(parse_markdown("1. first\n2. second"), vec![Block::Ordered(vec!["first".into(), "second".into()])]);
        assert_eq!(parse_markdown("> one\n> two"), vec![Block::Quote("one\ntwo".into())]);
        let b = parse_markdown("para\n\n---\n\nmore");
        assert!(matches!(b[..], [Block::Paragraph(_), Block::Hr, Block::Paragraph(_)]));
    }

    /// node: tests/widgets-markdown.test.ts:67-93
    #[test]
    fn inline() {
        let s = parse_inline("plain **bold** *italic* `code`");
        assert_eq!(
            s,
            vec![
                InlineSegment::plain("plain "),
                InlineSegment { text: "bold".into(), bold: true, ..Default::default() },
                InlineSegment::plain(" "),
                InlineSegment { text: "italic".into(), italic: true, ..Default::default() },
                InlineSegment::plain(" "),
                InlineSegment { text: "code".into(), code: true, ..Default::default() },
            ]
        );
        let s = parse_inline("see [docs](https://example.com) now");
        assert_eq!(
            s,
            vec![
                InlineSegment::plain("see "),
                InlineSegment { text: "docs".into(), url: Some("https://example.com".into()), ..Default::default() },
                InlineSegment::plain(" now"),
            ]
        );
        assert_eq!(parse_inline("nothing **here"), vec![InlineSegment::plain("nothing **here")]);
    }

    /// node: tests/widgets-markdown.test.ts:95-107
    #[test]
    fn render_smoke() {
        let t = crate::theme::COOL_BLUE;
        let rows = render_markdown(
            &t,
            "# Title\n\nsome **emphasis**\n\n- [ ] a task\n- [x] done\n\n```\ncode here\n```\n\n> a quote\n",
            None,
        );
        assert!(!rows.is_empty());
        let long = "one two three four five six seven eight nine ten eleven twelve";
        assert!(render_markdown(&t, long, Some(20)).len() > render_markdown(&t, long, None).len());
        let lines = markdown_lines(&t, "# Title\n\n- a", None);
        assert_eq!(lines[0].to_string(), "# Title");
        assert_eq!(lines[2].to_string(), "  • a");
        assert_eq!(wrap_line("aaaa bbbb cccc", Some(9)), vec!["aaaa bbbb", "cccc"]);
    }
}
