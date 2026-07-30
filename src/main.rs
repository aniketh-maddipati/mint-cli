mod agents;
mod app;
mod config;
mod logging;
mod session;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::execute;
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::app::{App, AppEvent};
use crate::config::Config;

type Term = Terminal<CrosstermBackend<Stdout>>;

#[tokio::main]
async fn main() -> Result<()> {
    let _guard = logging::init();
    let config = Config::load_or_default();

    let mut terminal = setup_terminal().context("setup terminal")?;
    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();

    spawn_input_reader(tx.clone());
    spawn_ticker(tx.clone());

    let app = App::new(config, tx);
    let result = run(&mut terminal, app, rx).await;

    restore_terminal(&mut terminal).ok();
    result
}

async fn run(terminal: &mut Term, mut app: App, mut rx: UnboundedReceiver<AppEvent>) -> Result<()> {
    terminal.draw(|f| ui::draw(f, &mut app))?;

    while let Some(event) = rx.recv().await {
        if let AppEvent::Input(Event::Key(k)) = &event {
            if k.kind == KeyEventKind::Release {
                continue;
            }
        }

        if let AppEvent::Input(Event::Resize(cols, rows)) = &event {
            terminal.resize(Rect::new(0, 0, *cols, *rows))?;
        }

        app.handle_event(event);
        if app.should_quit {
            app.save_all();
            break;
        }
        terminal.draw(|f| ui::draw(f, &mut app))?;
    }

    Ok(())
}

fn spawn_input_reader(tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let mut reader = EventStream::new();
        while let Some(Ok(event)) = reader.next().await {
            if let Event::Key(k) = &event {
                if k.code == KeyCode::Char('q') && k.modifiers.contains(KeyModifiers::CONTROL) {
                    let _ = tx.send(AppEvent::Input(event));
                    break;
                }
            }
            if tx.send(AppEvent::Input(event)).is_err() {
                break;
            }
        }
    });
}

fn spawn_ticker(tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(120));
        loop {
            interval.tick().await;
            if tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });
}

fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
