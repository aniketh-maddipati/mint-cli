use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use tui_term::widget::PseudoTerminal;

use crate::app::{App, Focus, PaneLayout};
use crate::pane::LayoutMode;

const ACCENT: Color = Color::Rgb(0x3d, 0xe0, 0xa0);
const ACCENT_DIM: Color = Color::Rgb(0x1f, 0x6e, 0x53);
const MUTED: Color = Color::Rgb(0x6b, 0x72, 0x80);
const HEADER_HEIGHT: u16 = 1;
const FOOTER_HEIGHT: u16 = 1;
const TIMELINE_HEIGHT: u16 = 6;

pub fn draw(f: &mut Frame, app: &mut App) {
    app.layout.panes.clear();

    let area = f.area();
    let mut constraints = vec![
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(3),
    ];
    if app.panes.timeline_visible {
        constraints.push(Constraint::Length(TIMELINE_HEIGHT));
    }
    constraints.push(Constraint::Length(FOOTER_HEIGHT));

    let sections = Layout::vertical(constraints).split(area);
    let header = sections[0];
    let body = sections[1];
    let (timeline, footer) = if app.panes.timeline_visible {
        (Some(sections[2]), sections[3])
    } else {
        (None, sections[2])
    };

    render_header(f, app, header);

    let pane_rects = app.panes.layout_rects(body);
    let mut layouts = Vec::new();
    for (pane_idx, area) in pane_rects {
        let inner = inner_rect(area);
        layouts.push(PaneLayout {
            pane_idx,
            area,
            inner,
        });
        let focused = app.panes.active == pane_idx && app.focus == Focus::Pty;
        render_pane(f, app, pane_idx, area, focused);
    }
    app.set_pane_layouts(layouts);
    app.ensure_visible_panes_started();
    app.resize_live_ptys();

    if let Some(timeline_area) = timeline {
        app.layout.timeline = timeline_area;
        render_timeline(f, app, timeline_area);
    }

    render_footer(f, app, footer);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let active = app.panes.active();
    let mode = app.layout_mode_label();
    let status = if active.is_running() { "running" } else { "starting…" };
    let line = Line::from(vec![
        Span::styled(" mint ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(" {} · {} · {} · {} ", app.project_label(), active.title(), mode, status),
            Style::default().fg(Color::White),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_pane(f: &mut Frame, app: &App, pane_idx: usize, area: Rect, focused: bool) {
    let pane = &app.panes.panes[pane_idx];
    let title = format!(" {} ", pane.title());
    let block = Block::bordered()
        .title(Span::styled(
            title,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .border_style(border_style(focused));

    match &pane.pty {
        Some(s) => {
            let screen = s.parser.screen();
            let term = PseudoTerminal::new(screen).block(block);
            f.render_widget(&term, area);
        }
        None => {
            let hint = Paragraph::new(Span::styled(
                "  starting…",
                Style::default().fg(MUTED),
            ))
            .block(block);
            f.render_widget(hint, area);
        }
    }
}

fn render_timeline(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Timeline;
    let block = Block::bordered()
        .title(Span::styled(
            " timeline ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .border_style(border_style(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = app
        .stages
        .iter()
        .enumerate()
        .map(|(i, stage)| {
            let marker = if i == app.active_stage { "▸" } else { " " };
            let branch = &stage.active_branch;
            let style = if i == app.active_stage {
                Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Span::styled(
                format!("{marker} {} · branch={branch}", stage.name),
                style,
            ))
        })
        .collect();

    f.render_widget(List::new(items), inner);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.status.is_empty() {
        "ready".to_string()
    } else {
        app.status.clone()
    };
    let split_hint = match app.panes.layout {
        LayoutMode::Single => "Ctrl+\\ split",
        LayoutMode::Split => "Ctrl+\\ single",
    };
    let line = Line::from(Span::styled(
        format!(
            "Ctrl+Q quit · F3 timeline · {} · [ ] pane · Ctrl+R restart · {status}",
            split_hint
        ),
        Style::default().fg(MUTED),
    ));
    f.render_widget(Paragraph::new(line), area);
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
