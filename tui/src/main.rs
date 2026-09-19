//! wanna — やりたいことリストの TUI クライアント

mod app;
mod config;
mod editor;
mod store;
mod sync;
mod ui;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use std::sync::mpsc;
use std::time::Duration;

fn main() -> Result<()> {
    let cfg = config::Config::load();
    let store = store::Store::open(&config::config_dir().join("cache.db"))?;

    let (to_app, from_sync) = mpsc::channel();
    let worker = match cfg.remote() {
        Some((server, token)) => {
            let (to_worker, worker_rx) = mpsc::channel();
            sync::spawn(wanna_core::client::Client::new(server, token)?, worker_rx, to_app);
            Some(to_worker)
        }
        None => None,
    };
    let mut app = app::App::new(store, worker)?;
    if !app.has_remote() {
        app.message = Some(format!(
            "サーバー未設定: {} に server と token を書くと同期します",
            config::config_dir().join("config.toml").display()
        ));
    }

    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app, &from_sync);
    ratatui::restore();
    res
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut app::App,
    from_sync: &mpsc::Receiver<sync::FromSync>,
) -> Result<()> {
    while !app.quit {
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.on_key(key);
                }
            }
        }
        if let Some(req) = app.editor.take() {
            // エディタに端末を明け渡し、戻ったら描画し直す
            ratatui::restore();
            let res = editor::edit(&req.text);
            crossterm::terminal::enable_raw_mode()?;
            crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
            terminal.clear()?;
            app.on_editor(req, res);
        }
        while let Ok(msg) = from_sync.try_recv() {
            app.on_sync(msg);
        }
        app.tick();
    }
    Ok(())
}
