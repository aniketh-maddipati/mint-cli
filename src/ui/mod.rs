pub mod scrubber;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, List, ListItem, Paragraph, Wrap};
use tui_term::widget::PseudoTerminal;

use crate::app::{Action, App, Focus, SessionRuntime};
use crate::session::{HttpProvider, PtyAgentKind, SessionKind};

const ACCENT: Color = Color::Rgb(0x3d, 0xe0, 0xa0);
const ACCENT_DIM: Color = Color::Rgb(0x1f, 0x6e, 0x53);
const MUTED: Color = Color::Rgb(0x6b, 0x72, 0x80);
const SESSION_WIDTH: u16 = 22;
const CONTROL_WIDTH: u16 = 44;

pub fn draw(f: &mut Frame, app: &mut App) {
    app.layout.sessions.clear();
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

    let cols = Layout::horizontal([
        Constraint::Length(SESSION_WIDTH),
        Constraint::Min(20),
        Constraint::Length(CONTROL_WIDTH),
    ])
    .split(body);
    let (sessions_area, output_area, control_area) = (cols[0], cols[1], cols[2]);

    render_header(f, app, header);
    render_sessions(f, app, sessions_area);
    render_output(f, app, output_area);
    render_controls(f, app, control_area);
    render_footer(f, app, footer);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .border_style(Style::default().fg(ACCENT_DIM))
        .title(Span::styled(
            " mint ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let active = app.active();
    let kind = session_kind_label(&active.session.kind);
    let status = session_status(app, app.active_idx);
    let title = format!("  {} · {} · {}  ", active.session.name, kind, status);
    f.render_widget(
        Paragraph::new(Span::styled(title, Style::default().fg(Color::White))),
        inner,
    );
}

fn render_sessions(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Sessions;
    let block = Block::bordered()
        .title(Span::styled(
            " sessions ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .border_style(border_style(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, rt)| {
            let dot = session_dot(app, i);
            let label = format!("{dot} {}", rt.session.name);
            let style = if i == app.active_idx {
                Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
            } else if focused && i == app.session_sel {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Span::styled(label, style))
        })
        .collect();

    f.render_widget(List::new(items), inner);

    // Hit targets for mouse selection.
    let mut y = inner.y;
    for _ in 0..app.sessions.len() {
        if y >= inner.y + inner.height {
            break;
        }
        app.layout.sessions.push(Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        });
        y += 1;
    }
}

fn render_output(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Output;
    let inner = inner_rect(area);

    app.set_output_size(inner.height.max(1), inner.width.max(1));
    app.layout.output = inner;

    let is_pty = app.sessions[app.active_idx].session.is_pty();
    let status = session_status(app, app.active_idx);
    let title = format!(" output · {} ", status);
    let block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .border_style(border_style(focused));

    if is_pty {
        render_pty_output(f, app, area, block);
    } else {
        render_http_output(f, app, area, block, inner);
    }
}

fn render_pty_output(f: &mut Frame, app: &App, area: Rect, block: Block) {
    let rt = app.active();
    match &rt.pty {
        Some(s) => {
            let screen = s.parser.screen();
            let term = PseudoTerminal::new(screen).block(block);
            f.render_widget(&term, area);
        }
        None => {
            let hint = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {} is not running.", rt.session.name),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(
                    "  Press Start (F3 controls) to launch in a PTY.",
                    Style::default().fg(MUTED),
                )),
            ])
            .block(block);
            f.render_widget(hint, area);
        }
    }
}

fn render_http_output(f: &mut Frame, app: &App, area: Rect, block: Block, inner: Rect) {
    let rt = app.active();
    let mut lines: Vec<Line> = Vec::new();

    for msg in &rt.session.messages {
        let (prefix, color) = match msg.role {
            crate::session::Role::User => ("user", ACCENT),
            crate::session::Role::Assistant => ("asst", Color::White),
            crate::session::Role::System => ("sys", MUTED),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{prefix}: "), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::raw(msg.content.clone()),
        ]));
        lines.push(Line::from(""));
    }

    if rt.http_running && !rt.streaming.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("asst: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw(rt.streaming.clone()),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ]));
    } else if rt.session.messages.is_empty() && rt.streaming.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No messages yet. F2 to focus, type a prompt, Enter to send.",
            Style::default().fg(MUTED),
        )));
    }

    let total = lines.len() as u16;
    let scroll = total.saturating_sub(inner.height);
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(para, area);
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

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let rt = app.active();
    let prompt_focused = app.focus == Focus::Output && rt.session.is_http();
    let block = Block::bordered().border_style(Style::default().fg(ACCENT_DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);

    let top = if rt.session.is_http() {
        let cursor = if prompt_focused { "▏" } else { "" };
        Line::from(vec![
            Span::styled("prompt ", Style::default().fg(ACCENT)),
            Span::styled(
                format!("{}{}", rt.session.draft_prompt, cursor),
                Style::default().fg(Color::White),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("status ", Style::default().fg(ACCENT)),
            Span::styled(app.status.clone(), Style::default().fg(Color::White)),
        ])
    };
    f.render_widget(Paragraph::new(top), lines[0]);

    let debug = debug_line(rt);
    f.render_widget(Paragraph::new(debug), lines[1]);
}

fn debug_line(rt: &SessionRuntime) -> Line<'static> {
    match &rt.last_run {
        Some(run) => {
            let err = run
                .error
                .as_deref()
                .map(|e| format!(" · err={e}"))
                .unwrap_or_default();
            let tag = run
                .tag
                .map(|t| format!(" · tag={t:?}"))
                .unwrap_or_default();
            Line::from(Span::styled(
                format!(
                    "run {} · {:?} · {}ms · model={}{}{}",
                    run.short_id(),
                    run.status,
                    run.duration_ms,
                    run.model,
                    tag,
                    err
                ),
                Style::default().fg(MUTED),
            ))
        }
        None => Line::from(Span::styled(
            format!(
                "model={} · branch={} · {}",
                if rt.session.model.is_empty() {
                    "—".to_string()
                } else {
                    rt.session.model.clone()
                },
                rt.session.active_branch,
                if rt.http_running {
                    "streaming…"
                } else {
                    "no runs yet"
                }
            ),
            Style::default().fg(MUTED),
        )),
    }
}

fn session_kind_label(kind: &SessionKind) -> &'static str {
    match kind {
        SessionKind::Pty {
            agent: PtyAgentKind::Claude,
        } => "Claude PTY",
        SessionKind::Pty {
            agent: PtyAgentKind::Codex,
        } => "Codex PTY",
        SessionKind::Http {
            provider: HttpProvider::Tinker,
        } => "Tinker",
        SessionKind::Http {
            provider: HttpProvider::LmStudio,
        } => "LM Studio",
    }
}

fn session_status(app: &App, idx: usize) -> &'static str {
    let rt = &app.sessions[idx];
    match rt.session.kind {
        SessionKind::Pty { .. } if rt.pty.is_some() => "running",
        SessionKind::Http { .. } if rt.http_running => "streaming",
        _ => "idle",
    }
}

fn session_dot(app: &App, idx: usize) -> &'static str {
    match session_status(app, idx) {
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
