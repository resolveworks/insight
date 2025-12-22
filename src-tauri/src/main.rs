// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(all(not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    insight_lib::run();
}
