// Keeps a console window from opening alongside the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Chrome starts the native-messaging relay with the extension's origin as an argument.
    if std::env::args().skip(1).any(|a| a.starts_with("chrome-extension://")) {
        std::process::exit(waddle_lib::native_host());
    }
    waddle_lib::run()
}
