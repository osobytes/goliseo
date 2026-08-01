#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
smoke_root="$(mktemp -d)"
trap 'rm -rf "$smoke_root"' EXIT

node --check "$project_root/scripts/webrtc_proof_host.js"
node --check "$project_root/scripts/webrtc_proof_runner.js"
node --check "$project_root/scripts/webrtc_proof_suite.js"
node --check "$project_root/scripts/webrtc_star_host.js"
node --check "$project_root/scripts/webrtc_star_smoke.js"
node --check "$project_root/scripts/browser_storage_host.js"
node --check "$project_root/scripts/browser_storage_smoke.js"
node "$project_root/scripts/browser_storage_smoke.js"
node "$project_root/scripts/webrtc_proof_smoke.js"
node "$project_root/scripts/webrtc_star_smoke.js"
python3 "$project_root/scripts/browser_matrix.py" --self-test

python3 - "$project_root" <<'PY'
import re
import sys
from pathlib import Path

# The channel configuration exists in two languages. Values match today; this
# pins them together so a change on one side cannot drift silently.
root = Path(sys.argv[1])
lua = (root / "game" / "transport" / "contract.lua").read_text(encoding="utf-8")
js = (root / "scripts" / "webrtc_star_host.js").read_text(encoding="utf-8")


def lua_number(name: str) -> int:
    match = re.search(rf"^contract\.{name} = (\d+)$", lua, re.MULTILINE)
    if not match:
        raise SystemExit(f"contract.lua does not define {name}")
    return int(match.group(1))


def js_number(name: str) -> int:
    match = re.search(rf"^  var {name} = (\d+);$", js, re.MULTILINE)
    if not match:
        raise SystemExit(f"webrtc_star_host.js does not define {name}")
    return int(match.group(1))


for lua_name, js_name in (
    # VERSION first, and it was missing until #316. The bounds below were pinned
    # but the envelope version was not, so bumping `contract.VERSION` without the
    # JS host drifted silently past this gate and surfaced two scripts later as
    # "addressed control send failed" -- a routing message for a version fault.
    ("VERSION", "VERSION"),
    ("MAX_GUESTS", "MAX_GUESTS"),
    ("MAX_PAYLOAD_BYTES", "MAX_PAYLOAD_BYTES"),
    ("MAX_QUEUE_LIMIT", "MAX_QUEUE_LIMIT"),
    ("DEFAULT_QUEUE_LIMIT", "DEFAULT_QUEUE_LIMIT"),
    ("MAX_PEER_ID_BYTES", "MAX_PEER_ID_BYTES"),
    ("DEFAULT_BUFFERED_AMOUNT_LIMIT", "DEFAULT_BUFFERED_AMOUNT_LIMIT"),
    ("MAX_BUFFERED_AMOUNT_LIMIT", "MAX_BUFFERED_AMOUNT_LIMIT"),
):
    if lua_number(lua_name) != js_number(js_name):
        raise SystemExit(
            f"transport bound drifted between Lua and JavaScript: {lua_name}"
        )

lua_control = re.search(r"control = \{ label = \"control\", ordered = (\w+)", lua)
lua_input = re.search(
    r"input = \{ label = \"input\", ordered = (\w+), max_retransmits = (\d+)", lua
)
js_control = re.search(r"control: \{ ordered: (\w+) \}", js)
js_input = re.search(r"input: \{ ordered: (\w+), maxRetransmits: (\d+) \}", js)
if not (lua_control and lua_input and js_control and js_input):
    raise SystemExit("channel configuration could not be read from both sources")
if lua_control.group(1) != js_control.group(1):
    raise SystemExit("control channel ordering drifted between Lua and JavaScript")
if lua_input.groups() != js_input.groups():
    raise SystemExit("input channel configuration drifted between Lua and JavaScript")
print("transport channel configuration parity: OK")
PY

first="$smoke_root/first"
second="$smoke_root/second"
"$project_root/scripts/web_build.sh" "$first"
"$project_root/scripts/web_build.sh" "$second"
node "$project_root/scripts/transport_bridge_smoke.js" "$first/player.js"

cmp "$first/goliseo.love" "$second/goliseo.love"

python3 - "$first" <<'PY'
import json
import sys
import zipfile
from pathlib import Path

artifact = Path(sys.argv[1])
required = {
    ".htaccess",
    "11.5/love.js",
    "11.5/love.wasm",
    "goliseo.love",
    "index.html",
    "lua/normalize1.lua",
    "lua/normalize2.lua",
    "manifest.json",
    "player.js",
    "style.css",
    "third_party/lovejs-player.js",
    "third_party/lovejs.LICENSE.txt",
    "webrtc-proof.html",
    "webrtc-proof.js",
    "webrtc-proof-suite.html",
    "webrtc-proof-suite.js",
}
missing = sorted(path for path in required if not (artifact / path).is_file())
if missing:
    raise SystemExit(f"missing browser artifact files: {', '.join(missing)}")

index = (artifact / "index.html").read_text(encoding="utf-8")
if 'player.js?g=goliseo.love&amp;v=11.5' not in index:
    raise SystemExit("index.html does not point at the packaged game and LÖVE 11.5")
loader = (artifact / "player.js").read_text(encoding="utf-8")
if "Promise.all(paths.map(fetch_binary))" not in loader:
    raise SystemExit("browser loader does not use direct deterministic asset loading")
if "page_query.get(\"arg\")" not in loader:
    raise SystemExit("browser loader does not read flow arguments from the page URL")
if "GoliseoBrowserStorage" not in loader:
    raise SystemExit("browser loader does not include the settings persistence host")
if 'page_query.get("storage") === "unavailable"' not in loader:
    raise SystemExit("browser loader does not expose the unavailable-storage diagnostic")
if "Module.FS.syncfs = function (_populate, callback)" in loader:
    raise SystemExit("browser loader still suppresses persistent filesystem synchronization")
for marker in (
    "window.__GOLISEO__",
    "console_entries",
    'mark("first_frame")',
    "GC_BROWSER|",
    "runtime_postrun",
):
    if marker not in loader:
        raise SystemExit(f"browser loader is missing compatibility marker: {marker}")
if "GoliseoTransportBridge" not in loader:
    raise SystemExit("browser loader does not include the transport bridge host")
for marker in ("GoliseoWebRTCProof", "RTCPeerConnection", "GC_WEBRTC"):
    if marker not in loader:
        raise SystemExit(f"browser loader is missing WebRTC proof marker: {marker}")
for marker in (
    "GoliseoStarTransport",
    'maxRetransmits: 0',
    "open_peer",
    "take_signal",
    "buffered_amount_limit",
):
    if marker not in loader:
        raise SystemExit(f"browser loader is missing host-star transport marker: {marker}")
# A shipped bundle must expose no channel-injection seam: it would let any
# co-resident script attach a handle to an open peer and forge traffic
# attributed to that peer id, bypassing the offer/answer/DTLS handshake.
for forbidden in ("attach_channel",):
    if forbidden in loader:
        raise SystemExit(f"browser loader exposes a transport test seam: {forbidden}")

style = (artifact / "style.css").read_text(encoding="utf-8")
for marker in (
    "GOLISEO preserves its 960x540 canvas aspect ratio",
    "#canvas:not(:fullscreen)",
    "width: min(100vw, 177.7777777778vh) !important",
    "height: min(100vh, 56.25vw) !important",
    "transform: translate(-50%, -50%)",
):
    if marker not in style:
        raise SystemExit(f"browser stylesheet is missing aspect-ratio rule: {marker}")

proof_page = (artifact / "webrtc-proof.html").read_text(encoding="utf-8")
proof_runner = (artifact / "webrtc-proof.js").read_text(encoding="utf-8")
for marker in ("signal-input", "signal-output", "start-traffic", "diagnostics"):
    if marker not in proof_page:
        raise SystemExit(f"WebRTC proof page is missing control: {marker}")
for marker in (
    "GoliseoWebRTCProof",
    "network_profile",
    "one_way_delay_ms",
    "input_loss_percent",
    "run_completed",
):
    if marker not in proof_runner:
        raise SystemExit(f"WebRTC proof runner is missing marker: {marker}")

proof_suite_page = (artifact / "webrtc-proof-suite.html").read_text(encoding="utf-8")
proof_suite = (artifact / "webrtc-proof-suite.js").read_text(encoding="utf-8")
for marker in ("baseline-host", "shaped-host", "mismatch-host", "run-suite"):
    if marker not in proof_suite_page:
        raise SystemExit(f"WebRTC proof suite page is missing control: {marker}")
for marker in ("GoliseoWebRTCProofSuite", "suite_complete", "mismatch_complete"):
    if marker not in proof_suite:
        raise SystemExit(f"WebRTC proof suite is missing marker: {marker}")

with zipfile.ZipFile(artifact / "goliseo.love") as package:
    if package.testzip() is not None:
        raise SystemExit("game package contains a corrupt entry")
    names = set(package.namelist())
    for path in (
        "conf.lua",
        "main.lua",
        "game/app.lua",
        "game/compatibility_metrics.lua",
        "game/transport.lua",
        "game/transport/browser_star.lua",
        "game/transport/fake_star.lua",
        "game/webrtc_proof.lua",
        "sim/match.lua",
    ):
        if path not in names:
            raise SystemExit(f"game package is missing {path}")

manifest = json.loads((artifact / "manifest.json").read_text(encoding="utf-8"))
if manifest["game_package"]["path"] != "goliseo.love":
    raise SystemExit("manifest game package path does not use the GOLISEO name")
if manifest["runtime"]["commit"] != "495c5eb7eb55b54aaadfc21405c58f50a6d819c4":
    raise SystemExit("manifest runtime commit is not pinned")
if not isinstance(manifest.get("source_dirty"), bool):
    raise SystemExit("manifest does not record whether the source tree was dirty")
print("browser artifact smoke: OK")
PY
