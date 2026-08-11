#!/usr/bin/env python3
"""Shared Selenium/WebDriver process management for the browser harnesses.

## Why this file exists

`scripts/browser_match_harness.py`, `scripts/browser_online_peers.py` and
`scripts/browser_render_bench.py` all launch a real Chrome or Firefox under
Selenium and need to tear it down without leaking a process group -- a
`driver.quit()` call can hang indefinitely against a wedged renderer, and a
plain `kill()` of the parent alone leaves orphaned child processes (the GPU
process chief among them) running after the harness exits. `launch` and
`quit_browser_bounded` are that logic, written once and imported by all
three, rather than three hand-mirrored copies that could drift (AGENTS.md
§9).

This previously lived inside `scripts/browser_determinism.py`, which ran the
frozen OMP-1 Lua determinism suite in a real browser. That suite and its
script were removed with the rest of the LÖVE/Lua implementation, but the
process-launch and bounded-teardown functions below have nothing to do with
Lua or with determinism specifically -- they are generic WebDriver process
management, and three still-active harnesses depend on them. This file is
that code, carried forward on its own.
"""

from __future__ import annotations

import os
import signal
import subprocess
import threading
import time
from pathlib import Path
from typing import Any


def driver_version(path: Path) -> str:
    result = subprocess.run(
        [str(path), "--version"],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    )
    return (result.stdout or result.stderr).strip()


def chrome_arguments(ci: bool) -> tuple[str, ...]:
    arguments = [
        "--headless=new",
        "--disable-dev-shm-usage",
        "--disable-extensions",
        "--no-default-browser-check",
        "--no-first-run",
    ]
    if ci:
        arguments.append("--no-sandbox")
    return tuple(arguments)


def firefox_arguments(ci: bool) -> tuple[str, ...]:
    if ci:
        return ()
    return ("-headless",)


def firefox_preferences(ci: bool) -> dict[str, bool | int]:
    preferences: dict[str, bool | int] = {
        "extensions.autoDisableScopes": 15,
        "extensions.enabledScopes": 0,
    }
    if ci:
        preferences.update(
            {
                "webgl.force-enabled": True,
                "gfx.webrender.software": True,
                "gfx.x11-egl.force-disabled": True,
            }
        )
    return preferences


def bounded_log_tail(path: Path, max_lines: int = 40, max_characters: int = 6000) -> str:
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as error:
        return f"<webdriver log unavailable: {error}>"
    tail = "\n".join(lines[-max_lines:])
    if len(tail) > max_characters:
        tail = "<truncated>\n" + tail[-max_characters:]
    return tail or "<webdriver log empty>"


def launch(browser_name: str, binary: Path, driver: Path, log: Path) -> Any:
    if browser_name == "chrome":
        from selenium import webdriver
        from selenium.webdriver.chrome.options import Options
        from selenium.webdriver.chrome.service import Service

        options = Options()
        options.binary_location = str(binary)
        for argument in chrome_arguments(os.environ.get("CI") == "true"):
            options.add_argument(argument)
        return webdriver.Chrome(
            service=Service(
                str(driver),
                log_output=str(log),
                popen_kw={"start_new_session": True},
            ),
            options=options,
        )

    from selenium import webdriver
    from selenium.webdriver.firefox.options import Options
    from selenium.webdriver.firefox.service import Service

    options = Options()
    options.binary_location = str(binary)
    ci = os.environ.get("CI") == "true"
    for argument in firefox_arguments(ci):
        options.add_argument(argument)
    for name, value in firefox_preferences(ci).items():
        options.set_preference(name, value)
    return webdriver.Firefox(
        service=Service(
            str(driver),
            log_output=str(log),
            popen_kw={"start_new_session": True},
        ),
        options=options,
    )


def wait_process(process: Any, timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            return True
        time.sleep(0.05)
    return process.poll() is not None


def process_group_alive(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    return True


def wait_group_gone(group_alive: Any, pgid: int, timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if not group_alive(pgid):
            return True
        time.sleep(0.05)
    return not group_alive(pgid)


def quit_browser_bounded(
    driver: Any,
    timeout_seconds: float = 30,
    term_wait_seconds: float = 5,
    kill_wait_seconds: float = 5,
    getpgid: Any = os.getpgid,
    killpg: Any = os.killpg,
    group_alive: Any = process_group_alive,
) -> dict[str, Any]:
    process = getattr(getattr(driver, "service", None), "process", None)
    if process is None or getattr(process, "pid", None) is None:
        raise RuntimeError("WebDriver service process is unavailable for bounded teardown")
    pgid = getpgid(process.pid)
    quit_errors: list[str] = []

    def quit_driver() -> None:
        try:
            driver.quit()
        except Exception as error:  # teardown must still reap the process group
            quit_errors.append(str(error))

    thread = threading.Thread(target=quit_driver, daemon=True)
    thread.start()
    thread.join(timeout_seconds)
    service_exited = wait_process(process, 1)
    group_exited = wait_group_gone(group_alive, pgid, 1)
    fallback = thread.is_alive() or not service_exited or not group_exited
    signals: list[str] = []
    if fallback:
        try:
            killpg(pgid, signal.SIGTERM)
            signals.append("TERM")
        except ProcessLookupError:
            pass
        wait_process(process, term_wait_seconds)
        if not wait_group_gone(group_alive, pgid, term_wait_seconds):
            try:
                killpg(pgid, signal.SIGKILL)
                signals.append("KILL")
            except ProcessLookupError:
                pass
            wait_process(process, kill_wait_seconds)
            if not wait_group_gone(group_alive, pgid, kill_wait_seconds):
                raise RuntimeError(
                    f"WebDriver process group {pgid} survived bounded TERM/KILL teardown"
                )
    return {
        "fallback": fallback,
        "process_group": pgid,
        "quit_error": quit_errors[0] if quit_errors else None,
        "service_exit_code": process.poll(),
        "signals": signals,
    }
