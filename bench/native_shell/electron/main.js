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

// Disable the frame-rate cap so the bench's own MessageChannel loop is not
// silently vsync-limited, matching `scripts/babylon_bench.py`'s reasoning.
app.commandLine.appendSwitch("disable-frame-rate-limit");
app.commandLine.appendSwitch("disable-gpu-vsync");

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
