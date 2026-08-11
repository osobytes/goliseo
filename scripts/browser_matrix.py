#!/usr/bin/env python3
"""Resolve a pinned browser binary + WebDriver pair via Selenium Manager.

## Why this file is now just one function

This used to be the driver for an OMP-0 evidence campaign against the LÖVE
web build: launching Chrome/Firefox, driving the title -> squad -> formation
-> tactic -> match -> result flow, and checking canvas geometry, pointer
alignment, memory growth, console output and a `manifest.json` the love.js
build produced. That build (and the LÖVE/Lua implementation it packaged) no
longer exists, so that entire campaign -- `run_flow`, `run_viewport`,
`run_firefox_flow`, `run_preflight`, `run_firefox_heap_companion`,
`validate_manifest`, `evidence_checks`, `main`, `self_test`, and every helper
that existed only to feed them -- was removed with it.

`resolve_assets` survives because it is still imported and used:
`scripts/browser_online_peers.py` calls it to resolve a Chrome or Firefox
binary/driver pair through Selenium Manager, exactly the same way this file
used to resolve its own. Nothing else in this file is reachable from outside
it any more, and nothing else in this file was reachable from `resolve_assets`
even before this cut -- it has no internal dependency on any of the removed
code.
"""

from __future__ import annotations

from pathlib import Path


def resolve_assets(browser_name: str, binary: Path | None, driver: Path | None) -> tuple[Path, Path]:
    try:
        from selenium.webdriver.common.selenium_manager import SeleniumManager
    except ImportError as error:
        raise RuntimeError("Selenium is required: python3 -m pip install --user selenium") from error

    if binary and not binary.is_file():
        raise RuntimeError(f"browser binary does not exist: {binary}")
    if driver and not driver.is_file():
        raise RuntimeError(f"driver binary does not exist: {driver}")
    if binary and driver:
        return binary.resolve(), driver.resolve()

    args = [
        "--browser",
        browser_name,
        "--browser-version",
        "stable",
        "--skip-driver-in-path",
    ]
    if binary:
        args.extend(["--browser-path", str(binary.resolve())])
    result = SeleniumManager().binary_paths(args)
    resolved_binary = binary or Path(result["browser_path"])
    resolved_driver = driver or Path(result["driver_path"])
    if not resolved_binary.is_file() or not resolved_driver.is_file():
        raise RuntimeError("Selenium Manager did not return usable browser and driver paths")
    return resolved_binary.resolve(), resolved_driver.resolve()
