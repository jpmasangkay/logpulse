mod app;
mod ui;

use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

use crate::app::{spawn_ingestion, App, AppMode, LogEntry};

#[derive(Parser, Debug)]
#[command(
    name = "logpulse",
    version,
    about = "Real-time terminal UI for log exploration and system monitoring"
)]
struct Cli {
    #[command(flatten)]
    source: SourceArgs,
}

#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
struct SourceArgs {
    /// Path to a log file to tail
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Run with built-in demo log generator
    #[arg(long)]
    demo: bool,

    /// Monitor this machine directly (live processes, CPU/RAM/disk/network, and system logs)
    #[arg(long)]
    system: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let mode = if cli.source.demo {
        AppMode::Demo
    } else if cli.source.system {
        AppMode::System
    } else if let Some(path) = cli.source.file {
        if !path.exists() {
            bail!("Log file does not exist: {}", path.display());
        }
        if path.is_dir() {
            bail!("Expected a log file, but '{}' is a directory", path.display());
        }
        File::open(&path)
            .with_context(|| format!("Unable to open log file: {}", path.display()))?;
        AppMode::File(path)
    } else {
        bail!("Either <FILE>, --demo, or --system must be provided");
    };

    let mut app = App::new(mode.clone())?;
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let worker = spawn_ingestion(mode, tx, Arc::clone(&stop));

    let mut terminal = ratatui::try_init().context("failed to initialize terminal")?;
    let run_result = run_event_loop(&mut terminal, &mut app, &rx);
    ratatui::restore();

    stop.store(true, Ordering::Relaxed);
    drop(rx);
    let _ = worker.join();

    run_result
}

fn run_event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &Receiver<LogEntry>,
) -> Result<()> {
    let tick_rate = Duration::from_secs(1);
    let mut last_tick = Instant::now();

    loop {
        app.drain_incoming(rx);

        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = Instant::now();
        }

        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(50))? {
            loop {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        app.handle_key(key);
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
                if app.should_quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            if app.should_quit {
                break;
            }
        }
    }

    Ok(())
}
