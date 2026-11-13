#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    if !rune::single_instance("com.chiteshvarun.jarvis") {
        return;
    }
    jarvis_app::run();
}
