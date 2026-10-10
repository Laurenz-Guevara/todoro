//! Notes' Markdown drawn for reading in the terminal (`v`): headings, lists
//! and task lists, quotes, code, tables, links and emphasis, wrapped to the
//! screen with hanging indents.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Inline code: a background as well as a colour, so it's still marked
/// (reversed) without colours.
const CODE: Style = Style::new().fg(Color::Yellow).bg(Color::Rgb(45, 45, 45));
const HEADING: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const LINK: Style = Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED);
const DIM: Style = Style::new().fg(Color::DarkGray);
const BULLETS: [&str; 3] = ["•", "◦", "▪"];

/// `markdown` as lines no wider than `width`.
pub fn render(markdown: &str, width: usize) -> Vec<Line<'static>> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let mut renderer = Renderer { width: width.max(1), ..Renderer::default() };
    for event in Parser::new_ext(markdown, options) {
        renderer.event(event);
    }
    renderer.flush();
    renderer.lines
}

/// Text in its styles.
type Styled = Vec<(String, Style)>;

/// A table row: each cell's text.
type Row = Vec<Styled>;

/// A block that other blocks sit inside, adding to the start of their lines.
enum Container {
    Quote,
    /// A list item, whose marker shows on its first line only.
    Item { marker: String, marker_style: Style, shown: bool, task_done: bool },
}

#[derive(Default)]
struct Renderer {
    width: usize,
    lines: Vec<Line<'static>>,
    containers: Vec<Container>,
    /// Each open list's next number, or `None` for bullets.
    lists: Vec<Option<u64>>,
    /// Inline styles open (emphasis, links, headings), innermost last.
    styles: Vec<Style>,
    /// Text of the block being read, waiting to be wrapped.
    inline: Vec<(String, Style)>,
    /// Whether the next block needs a blank line before it.
    gap: bool,
    /// The heading being read, to underline it.
    heading: Option<HeadingLevel>,
    /// Where the open link goes, to show after its text, and where in
    /// `inline` its text starts.
    link: Option<(String, usize)>,
    /// The code block being read.
    code: Option<String>,
    /// The table being read.
    table: Option<Vec<Row>>,
}

impl Renderer {
    fn style(&self) -> Style {
        self.styles.iter().fold(Style::new(), |style, &next| style.patch(next))
    }

    fn text(&mut self, text: &str, style: Style) {
        if let Some(code) = &mut self.code {
            code.push_str(text);
        } else {
            self.inline.push((text.to_string(), style));
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text, self.style()),
            Event::Code(code) => self.text(&code, self.style().patch(CODE)),
            Event::InlineMath(math) | Event::DisplayMath(math) => self.text(&math, self.style().patch(CODE)),
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html, self.style().patch(DIM)),
            Event::FootnoteReference(name) => self.text(&format!("[{name}]"), self.style().patch(DIM)),
            // Notes are typed a line at a time, so their lines stay lines, as
            // Obsidian shows them, rather than joining into one paragraph.
            Event::SoftBreak => self.text("\n", self.style()),
            Event::HardBreak => self.text("\n", self.style()),
            Event::Rule => {
                self.flush();
                self.start_block();
                let room = self.width.saturating_sub(self.prefix_width()).max(1);
                self.push_line(vec![Span::styled("─".repeat(room), DIM)]);
                self.gap = true;
            }
            Event::TaskListMarker(done) => {
                if let Some(Container::Item { marker, marker_style, task_done, .. }) = self.containers.last_mut() {
                    *marker = if done { "☑ " } else { "☐ " }.to_string();
                    *marker_style = if done { Style::new().fg(Color::Green) } else { Style::new() };
                    *task_done = done;
                }
                if done {
                    self.styles.push(DIM);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.start_block();
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.start_block();
                self.heading = Some(level);
                let style = match level {
                    HeadingLevel::H1 | HeadingLevel::H2 => HEADING,
                    _ => Style::new().add_modifier(Modifier::BOLD),
                };
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.start_block();
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.start_block();
                self.code = Some(String::new());
            }
            Tag::List(start) => {
                // Text of the item this list is inside goes first.
                self.flush();
                if self.lists.is_empty() {
                    self.start_block();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                if self.gap {
                    self.start_block();
                }
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        format!("{}. ", *n - 1)
                    }
                    _ => format!("{} ", BULLETS[depth % BULLETS.len()]),
                };
                let marker_style = Style::new().fg(Color::Cyan);
                self.containers.push(Container::Item { marker, marker_style, shown: false, task_done: false });
            }
            Tag::Emphasis => self.styles.push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.styles.push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.link = Some((dest_url.to_string(), self.inline.len()));
                self.styles.push(LINK);
            }
            Tag::Image { .. } => {
                self.text("[image: ", self.style().patch(DIM));
                self.styles.push(DIM);
            }
            Tag::Table(_) => {
                self.flush();
                self.start_block();
                self.table = Some(Vec::new());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.push(Vec::new());
                }
                if matches!(tag, Tag::TableHead) {
                    self.styles.push(Style::new().add_modifier(Modifier::BOLD));
                }
            }
            Tag::HtmlBlock => {
                self.flush();
                self.start_block();
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::HtmlBlock => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(level) => {
                let width = self.inline.iter().map(|(text, _)| text.width()).sum::<usize>();
                self.flush();
                self.styles.pop();
                self.heading = None;
                let rule = match level {
                    HeadingLevel::H1 => Some("━"),
                    HeadingLevel::H2 => Some("─"),
                    _ => None,
                };
                if let Some(rule) = rule {
                    let room = self.width.saturating_sub(self.prefix_width()).max(1);
                    self.push_line(vec![Span::styled(rule.repeat(width.clamp(1, room)), DIM)]);
                }
                self.gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                let room = self.width.saturating_sub(self.prefix_width() + 2).max(1);
                for line in code.strip_suffix('\n').unwrap_or(&code).split('\n') {
                    for part in hard_wrap(&line.replace('\t', "    "), room) {
                        self.push_line(vec![Span::styled("▏ ", DIM), Span::styled(part, Style::new().fg(Color::Yellow))]);
                    }
                }
                self.gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                self.gap = self.lists.is_empty();
            }
            TagEnd::Item => {
                self.flush();
                if let Some(Container::Item { shown: false, .. }) = self.containers.last() {
                    // An empty item still shows its marker.
                    self.push_line(Vec::new());
                }
                if let Some(Container::Item { task_done: true, .. }) = self.containers.pop() {
                    self.styles.pop();
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some((url, start)) = self.link.take() {
                    let text: String = self.inline[start.min(self.inline.len())..].iter().map(|(text, _)| text.as_str()).collect();
                    if !url.is_empty() && text != url && !url.starts_with('#') {
                        self.text(&format!(" ({url})"), self.style().patch(DIM));
                    }
                }
            }
            TagEnd::Image => {
                self.styles.pop();
                self.text("]", self.style().patch(DIM));
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(row) = self.table.as_mut().and_then(|table| table.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::TableHead => {
                self.styles.pop();
            }
            TagEnd::Table => {
                let rows = self.table.take().unwrap_or_default();
                self.table_lines(rows);
                self.gap = true;
            }
            _ => {}
        }
    }

    /// A blank line before a block, if one came before it.
    fn start_block(&mut self) {
        if self.gap && !self.lines.is_empty() {
            let prefix = self.prefix(false);
            let bar_only: Vec<Span<'static>> =
                prefix.into_iter().filter(|span| !span.content.trim().is_empty()).collect();
            self.lines.push(Line::from(bar_only));
        }
        self.gap = false;
    }

    /// What goes before a line inside the open containers: list markers on
    /// an item's first line (`first`), indents and quote bars otherwise.
    fn prefix(&self, first: bool) -> Vec<Span<'static>> {
        self.containers
            .iter()
            .map(|container| match container {
                Container::Quote => Span::styled("▌ ", DIM),
                Container::Item { marker, marker_style, shown, .. } if first && !shown => {
                    Span::styled(marker.clone(), *marker_style)
                }
                Container::Item { marker, .. } => Span::raw(" ".repeat(marker.width())),
            })
            .collect()
    }

    fn prefix_width(&self) -> usize {
        self.prefix(false).iter().map(|span| span.content.width()).sum()
    }

    /// Adds a line after the containers' prefix. The first line inside an
    /// item shows its marker.
    fn push_line(&mut self, spans: Vec<Span<'static>>) {
        let mut line = self.prefix(true);
        for container in &mut self.containers {
            if let Container::Item { shown, .. } = container {
                *shown = true;
            }
        }
        line.extend(spans);
        self.lines.push(Line::from(line));
    }

    /// Wraps the text read so far into lines.
    fn flush(&mut self) {
        if self.inline.is_empty() || self.table.is_some() {
            return;
        }
        let inline = std::mem::take(&mut self.inline);
        let room = self.width.saturating_sub(self.prefix_width()).max(1);
        for line in wrap_spans(&inline, room) {
            self.push_line(line);
        }
    }

    /// A table with its columns lined up, or each row on its own wrapped
    /// line if that's too wide.
    fn table_lines(&mut self, rows: Vec<Row>) {
        let width_of = |cell: &Styled| cell.iter().map(|(text, _)| text.width()).sum::<usize>();
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|col| rows.iter().filter_map(|row| row.get(col)).map(width_of).max().unwrap_or(0))
            .collect();
        let room = self.width.saturating_sub(self.prefix_width()).max(1);
        let fits = widths.iter().sum::<usize>() + 3 * columns.saturating_sub(1) <= room;
        for (n, row) in rows.iter().enumerate() {
            let mut spans: Vec<(String, Style)> = Vec::new();
            for (col, cell) in row.iter().enumerate() {
                if col > 0 {
                    spans.push((" │ ".into(), DIM));
                }
                spans.extend(cell.iter().cloned());
                if fits && col + 1 < row.len() {
                    spans.push((" ".repeat(widths[col] - width_of(cell)), Style::new()));
                }
            }
            if fits {
                self.push_line(spans.into_iter().map(|(text, style)| Span::styled(text, style)).collect());
            } else {
                for line in wrap_spans(&spans, room) {
                    self.push_line(line);
                }
            }
            if n == 0 && fits {
                let rule: Vec<String> = widths.iter().map(|&width| "─".repeat(width)).collect();
                self.push_line(vec![Span::styled(rule.join("─┼─"), DIM)]);
            }
        }
    }
}

/// Styled text wrapped at spaces into lines of at most `width` columns, with
/// runs of spaces made one, and `\n` starting a new line. Words too long for
/// a line are broken.
fn wrap_spans(spans: &[(String, Style)], width: usize) -> Vec<Vec<Span<'static>>> {
    let mut wrap = Wrap { width, lines: Vec::new(), line: Vec::new(), used: 0, space: None };
    // The word being read, which may change style partway through.
    let mut word: Vec<(String, Style)> = Vec::new();
    for (text, style) in spans {
        let mut run = String::new();
        for c in text.chars() {
            if c.is_whitespace() {
                if !run.is_empty() {
                    word.push((std::mem::take(&mut run), *style));
                }
                wrap.place(&mut word);
                if c == '\n' {
                    wrap.break_line();
                } else {
                    wrap.space = Some(*style);
                }
            } else {
                run.push(c);
            }
        }
        if !run.is_empty() {
            word.push((run, *style));
        }
    }
    wrap.place(&mut word);
    if !wrap.line.is_empty() {
        wrap.break_line();
    }
    wrap.lines
}

/// Lines being filled by `wrap_spans`.
struct Wrap {
    width: usize,
    lines: Vec<Vec<Span<'static>>>,
    line: Vec<Span<'static>>,
    /// Columns used on `line`.
    used: usize,
    /// A space waiting to go before the next word, in its style.
    space: Option<Style>,
}

impl Wrap {
    fn break_line(&mut self) {
        self.lines.push(std::mem::take(&mut self.line));
        self.used = 0;
        self.space = None;
    }

    /// Adds a word, on a new line if it doesn't fit on this one.
    fn place(&mut self, word: &mut Vec<(String, Style)>) {
        let word_width: usize = word.iter().map(|(text, _)| text.width()).sum();
        if word_width == 0 {
            return;
        }
        let gap = usize::from(self.space.is_some() && self.used > 0);
        if self.used > 0 && self.used + gap + word_width > self.width {
            self.break_line();
        }
        if let Some(style) = self.space.take()
            && self.used > 0
        {
            self.line.push(Span::styled(" ", style));
            self.used += 1;
        }
        for (text, style) in word.drain(..) {
            if self.used + text.width() <= self.width {
                self.used += text.width();
                self.line.push(Span::styled(text, style));
                continue;
            }
            // Too long for any line: break it wherever it reaches the edge.
            let mut part = String::new();
            for c in text.chars() {
                let w = c.width().unwrap_or(0);
                if self.used + w > self.width && self.used > 0 {
                    if !part.is_empty() {
                        self.line.push(Span::styled(std::mem::take(&mut part), style));
                    }
                    self.break_line();
                }
                part.push(c);
                self.used += w;
            }
            if !part.is_empty() {
                self.line.push(Span::styled(part, style));
            }
        }
    }
}

/// `text` cut into pieces at most `width` columns wide, keeping every
/// character (for code, where spaces matter).
fn hard_wrap(text: &str, width: usize) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width && used > 0 {
            parts.push(String::new());
            used = 0;
        }
        parts.last_mut().expect("never empty").push(c);
        used += w;
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each line, without styles or trailing spaces.
    fn plain(markdown: &str, width: usize) -> Vec<String> {
        render(markdown, width)
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_string())
            .collect()
    }

    /// The style of the first span with exactly this text.
    fn style_of(markdown: &str, text: &str) -> Style {
        render(markdown, 80)
            .iter()
            .flat_map(|line| line.spans.clone())
            .find(|span| span.content == text)
            .unwrap_or_else(|| panic!("no span {text:?}"))
            .style
    }

    #[test]
    fn paragraphs_wrap_at_spaces_with_a_blank_line_between() {
        assert_eq!(plain("one two three four\n\nfive", 9), ["one two", "three", "four", "", "five"]);
        // A single line break stays a line break, as typed.
        assert_eq!(plain("one\ntwo", 20), ["one", "two"]);
        // A hard break (two spaces) starts a new line.
        assert_eq!(plain("one  \ntwo", 20), ["one", "two"]);
    }

    #[test]
    fn long_words_and_wide_characters_are_broken_to_fit() {
        assert_eq!(plain("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        // Wide characters take two columns each.
        assert_eq!(plain("日本語です", 4), ["日本", "語で", "す"]);
        for line in render("日本語です and more", 5) {
            assert!(line.width() <= 5, "{line:?}");
        }
    }

    #[test]
    fn headings_are_bold_and_the_top_two_underlined() {
        assert_eq!(plain("# Title\n## Part\n### Small\ntext", 40), ["Title", "━━━━━", "", "Part", "────", "", "Small", "", "text"]);
        assert!(style_of("# Title", "Title").add_modifier.contains(Modifier::BOLD));
        assert!(style_of("### Small", "Small").add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn lists_have_markers_and_hanging_indents() {
        assert_eq!(plain("- one\n- two words here", 10), ["• one", "• two", "  words", "  here"]);
        assert_eq!(plain("3. three\n4. four", 20), ["3. three", "4. four"]);
        // Nested lists indent and change bullet.
        assert_eq!(plain("- outer\n  - inner\n- next", 20), ["• outer", "  ◦ inner", "• next"]);
        // An empty item still shows.
        assert_eq!(plain("-\n- two", 20), ["•", "• two"]);
    }

    #[test]
    fn task_lists_show_boxes_and_dim_done_ones() {
        assert_eq!(plain("- [ ] milk\n- [x] bread", 20), ["☐ milk", "☑ bread"]);
        assert_eq!(style_of("- [x] bread", "bread").fg, Some(Color::DarkGray));
        assert_eq!(style_of("- [ ] milk", "milk").fg, None);
    }

    #[test]
    fn quotes_have_a_bar_on_every_line() {
        assert_eq!(plain("> one two three\n>\n> four", 9), ["▌ one two", "▌ three", "▌", "▌ four"]);
    }

    #[test]
    fn code_keeps_its_spaces_and_lines() {
        assert_eq!(plain("```\nfn main() {\n    go();\n}\n```", 40), ["▏ fn main() {", "▏     go();", "▏ }"]);
        // Long lines are cut, not wrapped at spaces.
        assert_eq!(plain("```\nabc def\n```", 5), ["▏ abc", "▏  de", "▏ f"]);
        // Inline code has a background, to stay marked without colours.
        assert!(style_of("use `cargo`", "cargo").bg.is_some());
    }

    #[test]
    fn emphasis_links_and_strikethrough_are_styled() {
        assert!(style_of("*it*", "it").add_modifier.contains(Modifier::ITALIC));
        assert!(style_of("**bold**", "bold").add_modifier.contains(Modifier::BOLD));
        assert!(style_of("~~gone~~", "gone").add_modifier.contains(Modifier::CROSSED_OUT));
        assert!(style_of("**bold *both***", "both").add_modifier.contains(Modifier::BOLD | Modifier::ITALIC));
        // A link shows where it goes, unless that's its text.
        assert_eq!(plain("[docs](https://x.org)", 40), ["docs (https://x.org)"]);
        assert_eq!(plain("<https://x.org>", 40), ["https://x.org"]);
        assert!(style_of("[docs](https://x.org)", "docs").add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn tables_line_up_their_columns_or_wrap_when_too_wide() {
        let table = "| Item | Qty |\n|---|---|\n| milk | 2 |\n| bread | 10 |";
        assert_eq!(plain(table, 40), ["Item  │ Qty", "──────┼────", "milk  │ 2", "bread │ 10"]);
        assert!(style_of(table, "Item").add_modifier.contains(Modifier::BOLD));
        let narrow = plain(table, 8);
        assert_eq!(narrow[0], "Item │");
        assert!(narrow.iter().all(|line| line.width() <= 8), "{narrow:?}");
    }

    #[test]
    fn rules_span_the_width() {
        assert_eq!(plain("one\n\n---\n\ntwo", 5), ["one", "", "─────", "", "two"]);
    }

    #[test]
    fn plain_text_and_nothing_at_all() {
        assert_eq!(plain("just a note", 40), ["just a note"]);
        assert!(render("", 40).is_empty());
        // Never wider than asked, even at one column.
        for line in render("# Hi\n- a list item\n> quote\n\n```\ncode\n```", 1) {
            assert!(line.width() <= 3, "{line:?}");
        }
    }
}
