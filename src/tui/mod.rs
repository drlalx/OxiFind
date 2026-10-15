pub mod terminal;
pub mod app;
pub mod ui;

use std::error::Error;
use crate::engine::IndexScope;

pub fn run(scope: IndexScope) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut terminal = terminal::TerminalGuard::new()?;
    let mut app = app::TuiApp::new(scope);
    app.run(&mut terminal)
}
