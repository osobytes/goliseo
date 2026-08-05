// Electron shell for the #329 native-route measurement.
//
// This is deliberately the smallest window that can host the #341 Babylon
// scene: one BrowserWindow, one URL, no menu, no preload. The measurement is
// about what the *shell* costs, so anything this file adds is cost the
// comparison would have to subtract again.
//
// The page reports its own marks over HTTP to the runner that served it (see
// `scripts/native_shell_bench.py`), so the shell needs no IPC of its own and
// the Electron and Tauri shells stay comparable line for line.

const { app, BrowserWindow } = require("electron");

const url = process.argv.find((a) => a.startsWith("http://") || a.startsWith("https://"));
const width = Number(process.env.GC_SHELL_WIDTH || 1024);
const height = Number(process.env.GC_SHELL_HEIGHT || 640);

// NO vsync or frame-rate switches here, deliberately.
//
// An earlier version set `--disable-frame-rate-limit` and `--disable-gpu-vsync`
// and justified them as "matching `scripts/babylon_bench.py`'s reasoning". That
// precedent did not exist -- `babylon_bench.py` sets no such flags -- and worse,
// the Tauri shell has no equivalent to set, because WebKitGTK exposes none. The
// flags therefore gave one side of a two-sided comparison an anti-vsync control
// the other side could not have, sitting directly underneath `frame_p50` and
// `frame_p95`, which are precisely the samples compositor pacing leaks into.
//
// Pacing is now controlled identically for both shells from outside them, by
// the driver-level environment variables `scripts/native_shell_bench.py` sets
// in `launch_shell()`. Whatever they do, they do to both.

function createWindow() {
    const win = new BrowserWindow({
        width,
        height,
        show: true,
        autoHideMenuBar: true,
        webPreferences: {
            // No node integration: the page is the untouched bench page.
            nodeIntegration: false,
            contextIsolation: true,
            backgroundThrottling: false,
        },
    });
    win.setMenu(null);
    if (!url) {
        console.error("native_shell electron: no http(s) URL argument given");
        app.exit(2);
        return;
    }
    win.loadURL(url);
}

app.whenReady().then(createWindow);

app.on("window-all-closed", () => app.quit());
