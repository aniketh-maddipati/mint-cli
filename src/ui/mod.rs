pub mod scrubber;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, Paragraph, Wrap};
use tui_term::widget::PseudoTerminal;

use crate::app::{Action, AgentKind, App, Focus};

const ACCENT: Color = Color::Rgb(0x3d, 0xe0, 0xa0);
const ACCENT_DIM: Color = Color::Rgb(0x1f, 0x6e, 0x53);
const MUTED: Color = Color::Rgb(0x6b, 0x72, 0x80);
const CONTROL_WIDTH: u16 = 46;

pub fn draw(f: &mut Frame, app: &mut App) {
    app.layout.tabs.clear();
    app.layout.buttons.clear();
    app.layout.scrubber_bars.clear();

    let area = f.area();
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(4),
    ])
    .split(area);
    let (header, body, footer) = (rows[0], rows[1], rows[2]);

    let cols = Layout::horizontal([Constraint::Min(20), Constraint::Length(CONTROL_WIDTH)]).split(body);
    let (output_area, control_area) = (cols[0], cols[1]);

    render_header(f, app, header);
    render_output(f, app, output_area);
    render_controls(f, app, control_area);
    render_footer(f, app, footer);
}

fn render_header(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_style(Style::default().fg(ACCENT_DIM))
        .title(Span::styled(" mint ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut x = inner.x + 1;
    let y = inner.y;
    for (i, kind) in AgentKind::ALL.iter().enumerate() {
        let active = *kind == app.active;
        let dot = state_dot(app, *kind);
        let label = format!("  {} {} {}  ", i + 1, kind.title(), dot);
        let w = label.chars().count() as u16;
        let rect = Rect {
            x,
            y,
            width: w.min(inner.width.saturating_sub(x - inner.x)),
            height: 1,
        };

        let style = if active {
            Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        f.render_widget(Paragraph::new(Span::styled(label, style)), rect);

        app.layout.tabs.push(rect);
        x += w + 1;
        if x >= inner.x + inner.width {
            break;
        }
    }
}

fn render_output(f: &mut Frame, app: &mut App, area: Rect) {
    let active = app.active;
    let focused = app.focus == Focus::Output;
    let inner = inner_rect(area);

    app.set_output_size(inner.height.max(1), inner.width.max(1));
    app.layout.output = inner;

    let status = agent_status(app, active);
    let title = format!(" {} · {} ", active.title(), status);
    let block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .border_style(border_style(focused));

    match active {
        AgentKind::Claude | AgentKind::Codex => match app.pty(active) {
            Some(s) => {
                let screen = s.parser.screen();
                let term = PseudoTerminal::new(screen).block(block);
                f.render_widget(&term, area);
            }
            None => {
                let hint = Paragraph::new(vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        format!("  {} is not running.", active.title()),
                        Style::default().fg(Color::White),
                    )),
                    Line::from(Span::styled(
                        "  Press Start (F3 controls) to launch it in a PTY.",
                        Style::default().fg(MUTED),
                    )),
                ])
                .block(block);
                f.render_widget(hint, area);
            }
        },
        AgentKind::LmStudio => {
            let total = app.lm_output.lines().count() as u16;
            let scroll = total.saturating_sub(inner.height);
            let body = if app.lm_output.is_empty() {
                "  No output yet. Focus this pane (F2), type a prompt, press Enter.".to_string()
            } else {
                app.lm_output.clone()
            };
            let para = Paragraph::new(body)
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0));
            f.render_widget(para, area);
        }
    }
}

fn render_controls(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .title(Span::styled(
            " controls ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .border_style(border_style(app.focus == Focus::Controls));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sections = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(inner);
    render_buttons(f, app, sections[0]);
    render_scrubbers(f, app, sections[1]);
}

fn render_buttons(f: &mut Frame, app: &mut App, area: Rect) {
    let count = Action::ALL.len() as u32;
    let constraints: Vec<Constraint> = (0..count).map(|_| Constraint::Ratio(1, count)).collect();
    let cells = Layout::horizontal(constraints).spacing(1).split(Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: 1,
    });

    for (i, action) in Action::ALL.iter().enumerate() {
        let selected = app.focus == Focus::Controls && app.button_sel == i;
        let style = if selected {
            Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(Color::Rgb(0x24, 0x2a, 0x33))
        };
        let label = format!(" {} ", action.label());
        let para = Paragraph::new(Span::styled(label, style)).alignment(Alignment::Center);
        f.render_widget(para, cells[i]);
        app.layout.buttons.push(cells[i]);
    }
}

fn render_scrubbers(f: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let per = 3u16;
    for (i, s) in app.scrubbers.iter().enumerate() {
        let y = area.y + (i as u16) * per;
        if y + 1 >= area.y + area.height {
            break;
        }
        let selected = app.focus == Focus::Scrubbers && app.scrubber_sel == i;

        let label_style = if selected {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let label_line = Line::from(vec![
            Span::styled(
                if selected { "▸ " } else { "  " },
                Style::default().fg(ACCENT),
            ),
            Span::styled(s.label.clone(), label_style),
            Span::raw("  "),
            Span::styled(
                s.value_string(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
        ]);
        f.render_widget(
            Paragraph::new(label_line),
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
        );

        let bar_rect = Rect {
            x: area.x,
            y: y + 1,
            width: area.width,
            height: 1,
        };
        let gauge = Gauge::default()
            .ratio(s.ratio())
            .label("")
            .gauge_style(Style::default().fg(if selected { ACCENT } else { ACCENT_DIM }))
            .use_unicode(true);
        f.render_widget(gauge, bar_rect);
        app.layout.scrubber_bars.push(bar_rect);
    }
}

fn render_footer(f: &mut Frame, app: &mut App, area: Rect) {
    let prompt_focused = app.focus == Focus::Output && app.active == AgentKind::LmStudio;
    let block = Block::bordered().border_style(Style::default().fg(ACCENT_DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);

    let prompt_label = if app.active == AgentKind::LmStudio {
        let cursor = if prompt_focused { "▏" } else { "" };
        Line::from(vec![
            Span::styled("prompt ", Style::default().fg(ACCENT)),
            Span::styled(
                format!("{}{}", app.lm_prompt, cursor),
                Style::default().fg(Color::White),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("status ", Style::default().fg(ACCENT)),
            Span::styled(app.status.clone(), Style::default().fg(Color::White)),
        ])
    };
    f.render_widget(Paragraph::new(prompt_label), lines[0]);

    let help = Line::from(Span::styled(
        "Ctrl+Q quit   F2 output   F3 controls   F4 scrubbers   1/2/3 agent   ←/→ adjust",
        Style::default().fg(MUTED),
    ));
    f.render_widget(Paragraph::new(help), lines[1]);
}

fn agent_status(app: &App, kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude if app.claude.is_some() => "running",
        AgentKind::Codex if app.codex.is_some() => "running",
        AgentKind::LmStudio if app.lm_streaming => "streaming",
        _ => "idle",
    }
}

fn state_dot(app: &App, kind: AgentKind) -> &'static str {
    match agent_status(app, kind) {
        "running" | "streaming" => "●",
        _ => "○",
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ACCENT_DIM)
    }
}

fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}
