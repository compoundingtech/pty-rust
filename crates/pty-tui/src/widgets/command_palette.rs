//! Command palette (`src/tui/widgets/command-palette.ts`): a fuzzy-ranked
//! action runner. Keys: printable and editing keys edit the query (and
//! reset the selection), `up`/`down` walk the ranked list, `return` runs
//! the selected command, `escape` cancels. The render is a panel with the
//! `  > query█` line and up to `limit` (default 10) matches, `▸` on the
//! selected one, or `  no matches`.

use std::rc::Rc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::panel::Panel;
use crate::fuzzy::fuzzy_match;
use crate::input::KeyEvent;
use crate::line_edit::{TextFieldState, apply_text_key};
use crate::theme::{BoxStyle, Color, Theme};

/// `Command` (`command-palette.ts:14-25`).
#[derive(Clone)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub hint: Option<String>,
    pub keywords: Vec<String>,
    pub run: Rc<dyn Fn()>,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("hint", &self.hint)
            .field("keywords", &self.keywords)
            .finish()
    }
}

impl Command {
    pub fn new(id: impl Into<String>, label: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Command {
            id: id.into(),
            label: label.into(),
            hint: None,
            keywords: Vec::new(),
            run: Rc::new(run),
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn keywords(mut self, keywords: &[&str]) -> Self {
        self.keywords = keywords.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Invoke the command.
    pub fn run(&self) {
        (self.run)()
    }
}

/// `CommandPaletteState` (`command-palette.ts:27-30`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandPaletteState {
    pub query: TextFieldState,
    pub selected: usize,
}

impl CommandPaletteState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A ranked match.
#[derive(Debug, Clone)]
pub struct RankedCommand<'a> {
    pub cmd: &'a Command,
    pub score: i64,
}

/// Filter and rank by fuzzy match over label + hint + keywords; the empty
/// query returns everything in order (`filterCommands`).
pub fn filter_commands<'a>(commands: &'a [Command], query: &str) -> Vec<RankedCommand<'a>> {
    let q = query.trim();
    if q.is_empty() {
        return commands.iter().map(|cmd| RankedCommand { cmd, score: 0 }).collect();
    }
    let mut ranked: Vec<RankedCommand<'a>> = commands
        .iter()
        .filter_map(|cmd| {
            let mut hay = vec![cmd.label.clone(), cmd.hint.clone().unwrap_or_default()];
            hay.extend(cmd.keywords.iter().cloned());
            fuzzy_match(q, &hay.join(" ")).map(|score| RankedCommand { cmd, score })
        })
        .collect();
    ranked.sort_by_key(|r| std::cmp::Reverse(r.score));
    ranked
}

/// What a key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteAction {
    Run,
    Cancel,
    Edited,
    Moved,
    None,
}

/// `handleCommandPaletteKey` result.
#[derive(Debug, Clone)]
pub struct PaletteKeyResult<'a> {
    pub state: CommandPaletteState,
    pub action: PaletteAction,
    /// The command to run when `action == Run`.
    pub command: Option<&'a Command>,
}

/// `handleCommandPaletteKey` (`command-palette.ts:63-98`).
pub fn handle_command_palette_key<'a>(
    state: &CommandPaletteState,
    commands: &'a [Command],
    key: &KeyEvent,
) -> PaletteKeyResult<'a> {
    let result = |state: CommandPaletteState, action: PaletteAction| PaletteKeyResult {
        state,
        action,
        command: None,
    };
    if key.name == "escape" {
        return result(state.clone(), PaletteAction::Cancel);
    }
    let ranked = filter_commands(commands, &state.query.text);
    match key.name.as_str() {
        "up" => {
            return result(
                CommandPaletteState {
                    selected: state.selected.saturating_sub(1),
                    ..state.clone()
                },
                PaletteAction::Moved,
            );
        }
        "down" => {
            return result(
                CommandPaletteState {
                    selected: (state.selected + 1).min(ranked.len().saturating_sub(1)),
                    ..state.clone()
                },
                PaletteAction::Moved,
            );
        }
        "return" => {
            return match ranked.get(state.selected) {
                Some(r) => PaletteKeyResult {
                    state: state.clone(),
                    action: PaletteAction::Run,
                    command: Some(r.cmd),
                },
                None => result(state.clone(), PaletteAction::None),
            };
        }
        _ => {}
    }
    match apply_text_key(&state.query, key) {
        Some(query) => result(CommandPaletteState { query, selected: 0 }, PaletteAction::Edited),
        None => result(state.clone(), PaletteAction::None),
    }
}

/// The palette body lines: the query line then the matches
/// (`renderCommandPalette` rows, `command-palette.ts:102-135`).
pub fn command_palette_lines(
    theme: &Theme,
    state: &CommandPaletteState,
    commands: &[Command],
    limit: usize,
) -> Vec<Line<'static>> {
    let accent_bold = Style::default()
        .fg(theme.color(Color::Accent))
        .add_modifier(Modifier::BOLD);
    let accent = Style::default().fg(theme.color(Color::Accent));
    let primary = Style::default().fg(theme.color(Color::Primary));
    let muted = Style::default().fg(theme.color(Color::Muted));
    let dim = muted.add_modifier(Modifier::DIM);
    let mut lines = vec![Line::from(vec![
        Span::styled("  > ", accent_bold),
        Span::styled(state.query.text.clone(), primary),
        Span::styled("\u{2588}", accent),
    ])];
    let ranked = filter_commands(commands, &state.query.text);
    let visible = &ranked[..ranked.len().min(limit)];
    if visible.is_empty() {
        lines.push(Line::from(Span::styled("  no matches", dim)));
    } else {
        for (i, r) in visible.iter().enumerate() {
            let selected = i == state.selected;
            let mut spans = vec![if selected {
                Span::styled("  \u{25b8} ", accent_bold)
            } else {
                Span::styled("    ", muted)
            }];
            spans.push(Span::styled(
                r.cmd.label.clone(),
                if selected { accent_bold } else { primary },
            ));
            if let Some(h) = &r.cmd.hint {
                spans.push(Span::styled(format!("  {h}"), dim));
            }
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// The palette as a panel widget (`renderCommandPalette`).
pub struct CommandPalette<'a> {
    pub state: &'a CommandPaletteState,
    pub commands: &'a [Command],
    pub theme: Theme,
    pub box_style: BoxStyle,
    pub title: String,
    pub limit: usize,
}

impl<'a> CommandPalette<'a> {
    pub fn new(state: &'a CommandPaletteState, commands: &'a [Command], theme: Theme, box_style: BoxStyle) -> Self {
        CommandPalette {
            state,
            commands,
            theme,
            box_style,
            title: "command palette".into(),
            limit: 10,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

impl Widget for CommandPalette<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Panel::new(self.theme, self.box_style)
            .title(self.title.clone())
            .render(area, buf);
        let inner = Panel::inner(area);
        for (i, line) in command_palette_lines(&self.theme, self.state, self.commands, self.limit)
            .iter()
            .enumerate()
        {
            if i as u16 >= inner.height {
                break;
            }
            buf.set_line(inner.x, inner.y + i as u16, line, inner.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<Command> {
        vec![
            Command::new("open", "Open file", || {}).hint("in the editor").keywords(&["edit"]),
            Command::new("save", "Save", || {}).hint("current file"),
            Command::new("quit", "Quit", || {}).keywords(&["exit"]),
            Command::new("new", "New reminder", || {}),
        ]
    }

    fn ids<'a>(r: &[RankedCommand<'a>]) -> Vec<&'a str> {
        r.iter().map(|r| r.cmd.id.as_str()).collect()
    }

    /// node: tests/widgets-command-palette.test.ts:20-41
    #[test]
    fn filtering() {
        let c = commands();
        assert_eq!(ids(&filter_commands(&c, "")), vec!["open", "save", "quit", "new"]);
        assert!(ids(&filter_commands(&c, "ex")).contains(&"quit"));
        assert!(ids(&filter_commands(&c, "edi")).contains(&"open"));
        assert_eq!(filter_commands(&c, "sa")[0].cmd.id, "save");
        assert!(filter_commands(&c, "xyzq").is_empty());
    }

    /// node: tests/widgets-command-palette.test.ts:43-76
    #[test]
    fn keys() {
        let c = commands();
        let s0 = CommandPaletteState::new();
        let r = handle_command_palette_key(
            &CommandPaletteState { selected: 3, ..s0.clone() },
            &c,
            &KeyEvent::printable("o"),
        );
        assert_eq!(r.action, PaletteAction::Edited);
        assert_eq!(r.state.query.text, "o");
        assert_eq!(r.state.selected, 0);
        let d1 = handle_command_palette_key(&s0, &c, &KeyEvent::named("down"));
        assert_eq!((d1.state.selected, d1.action), (1, PaletteAction::Moved));
        let d2 = handle_command_palette_key(&d1.state, &c, &KeyEvent::named("down"));
        assert_eq!(d2.state.selected, 2);
        let up = handle_command_palette_key(&d2.state, &c, &KeyEvent::named("up"));
        assert_eq!(up.state.selected, 1);
        assert_eq!(handle_command_palette_key(&s0, &c, &KeyEvent::named("up")).state.selected, 0);
        let r = handle_command_palette_key(&s0, &c, &KeyEvent::named("return"));
        assert_eq!(r.action, PaletteAction::Run);
        assert_eq!(r.command.unwrap().id, "open");
        assert_eq!(handle_command_palette_key(&s0, &c, &KeyEvent::named("escape")).action, PaletteAction::Cancel);
    }

    /// node: tests/widgets-command-palette.test.ts:78-94
    #[test]
    fn rendering() {
        let c = commands();
        let t = crate::theme::COOL_BLUE;
        let lines = command_palette_lines(&t, &CommandPaletteState::new(), &c, 2);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].to_string(), "  > █");
        assert_eq!(lines[1].to_string(), "  ▸ Open file  in the editor");
        let s = CommandPaletteState {
            query: TextFieldState::new("xyzq"),
            selected: 0,
        };
        let lines = command_palette_lines(&t, &s, &c, 10);
        assert!(lines[1].to_string().contains("no matches"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 6));
        CommandPalette::new(&CommandPaletteState::new(), &c, t, BoxStyle::Rounded)
            .limit(2)
            .render(buf.area, &mut buf);
        let row0: String = (0..40).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row0.contains("command palette"));
    }
}
