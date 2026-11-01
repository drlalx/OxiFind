pub mod app;
pub mod terminal;
pub mod ui;

use crate::engine::IndexScope;
use std::error::Error;

pub fn run(scope: IndexScope) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut terminal = terminal::TerminalGuard::new()?;
    let mut app = app::TuiApp::new(scope);
    app.run(&mut terminal)
}
