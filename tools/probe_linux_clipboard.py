#!/usr/bin/env python3
"""Read-only P03 diagnostic: inspect session/interfaces, never read clipboard bytes.

Interface visibility alone is NOT proof of an authorized portal session or native
clipboard support. No portal session is opened and no setting is changed here.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

SESSION_KEYS = {"DISPLAY", "WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_SESSION_TYPE"}


def run(command, env=None, timeout=4):
    try:
        result = subprocess.run(command, env=env, capture_output=True, timeout=timeout, check=False)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired as error:
        return "timeout", error.stdout or b"", error.stderr or b""
    except OSError:
        return "unavailable", b"", b""


def probe():
    if sys.platform != "linux":
        return {"platform": sys.platform, "status": "linux-only diagnostic", "clipboard_ready": "not-tested"}
    env = dict(os.environ)
    code, data, _ = run(["systemctl", "--user", "show-environment"])
    # Whitelist only desktop discovery fields; never report the user's environment.
    imported = []
    if code == 0:
        for line in data.decode(errors="replace").splitlines():
            key, separator, value = line.partition("=")
            if separator and key in SESSION_KEYS and key not in env:
                env[key] = value
                imported.append(key)
    _, portal, _ = run(["gdbus", "introspect", "--session", "--dest", "org.freedesktop.portal.Desktop",
                       "--object-path", "/org/freedesktop/portal/desktop"], env=env)
    interfaces = [name for name in ("Clipboard", "RemoteDesktop", "InputCapture")
                  if f"interface org.freedesktop.portal.{name} {{".encode() in portal]
    clipboard_code, _, debug = run(["wl-paste", "--list-types"], env={**env, "WAYLAND_DEBUG": "client"})
    advertised = set(re.findall(rb'global\([^\n]*?"([^"]+)"', debug))
    controls = [name for name in ("ext_data_control_manager_v1", "zwlr_data_control_manager_v1")
                if name.encode() in advertised]
    runtime = env.get("XDG_RUNTIME_DIR", "")
    display = env.get("WAYLAND_DISPLAY", "")
    return {
        "platform": "linux",
        "session_type": env.get("XDG_SESSION_TYPE", "unknown"),
        "desktop": env.get("XDG_CURRENT_DESKTOP", "unknown"),
        "session_fields_imported_from_user_manager": sorted(imported),
        "wayland_socket_present": bool(runtime and display and (Path(runtime) / display).exists()),
        "portal_interfaces_visible": interfaces,
        "wl_paste_installed": shutil.which("wl-paste") is not None,
        "wl_paste_metadata_probe": clipboard_code,
        "registry_data_control_advertised": controls,
        "clipboard_ready": "not-tested",
        "limitation": "No clipboard content read/write or authorized portal session tested.",
    }


if __name__ == "__main__":
    print(json.dumps(probe(), ensure_ascii=False, indent=2))
