// Always a windowed app: no console window, also in debug builds (the app shows the server log itself).
#![windows_subsystem = "windows"]

fn main() {
    wridge_lib::run();
}
