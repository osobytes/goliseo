// Tauri shell for the #329 native-route measurement.
//
// The counterpart of `bench/native_shell/electron/main.js`, kept as close to it
// as two different frameworks allow: one window, one URL taken from argv, no
// menu, no IPC. On Linux this window is WebKitGTK, which is the whole point --
// it is the engine a Tauri build actually ships here, and #341 measured Babylon
// only under V8 and SpiderMonkey.
//
// The URL is read at runtime rather than baked into `tauri.conf.json` because
// the runner serves the bench page on an ephemeral port.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Url, WebviewUrl, WebviewWindowBuilder};

fn main() {
    let url = std::env::args()
        .find(|a| a.starts_with("http://") || a.starts_with("https://"))
        .unwrap_or_else(|| {
            eprintln!("native_shell tauri: no http(s) URL argument given");
            std::process::exit(2);
        });
    let parsed: Url = url.parse().unwrap_or_else(|err| {
        eprintln!("native_shell tauri: unparseable URL {url}: {err}");
        std::process::exit(2);
    });

    let width: f64 = std::env::var("GC_SHELL_WIDTH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024.0);
    let height: f64 = std::env::var("GC_SHELL_HEIGHT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(640.0);

    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(parsed.clone()))
                .title("goliseo native shell")
                .inner_size(width, height)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
