/// tuiOS — Terminal UI Experience (Rust/ratatui port)
/// Entry point — mirrors main.py

mod tuios;
mod settings;
mod touchscreen;
mod dashboards;
mod applications;
mod utilities;
mod app_store;
mod task_manager;

fn main() {
    tuios::app::main();
}
