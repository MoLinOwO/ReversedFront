#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if rf_desktop::updater::run_standalone_update_if_requested() {
        return;
    }
    rf_desktop::run();
}
