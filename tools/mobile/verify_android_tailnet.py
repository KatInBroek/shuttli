#!/usr/bin/env python3
"""Coordinate opt-in emulator/desktop tests using an external, private configuration."""
import argparse
import hashlib
import io
import ipaddress
import json
import os
from pathlib import Path
import queue
import re
import socket
import subprocess
import threading
import time
import uuid


def main():
    if not __debug__:
        raise RuntimeError("Run without Python optimization: test guards use assertions")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    profile = Path(config["profile"]).resolve()
    # Never clear a normal user's history based only on a supplied profile path.
    assert (profile / ".tailnet-test-profile").read_text().strip() == "disposable"
    serial = config["emulator_serial"]
    assert re.fullmatch(r"emulator-\d+", serial), "Only an explicitly selected emulator is supported"
    phone_id = config["phone_id"]
    desktop_id = config["desktop_id"]
    assert all(re.fullmatch(r"[0-9a-f]{64}", value) for value in (phone_id, desktop_id))
    phone_ip = ipaddress.IPv4Address(config["phone_ip"])
    assert phone_ip in ipaddress.IPv4Network("100.64.0.0/10")
    second = config.get("second_peer")
    if second:
        assert re.fullmatch(r"[0-9a-f]{64}", second["id"])
        assert isinstance(second["prepare_command"], list) and second["prepare_command"]
    output = Path(config["output"]).resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=True)
    token = uuid.uuid4().hex[:12]
    env = dict(os.environ, **config.get("environment", {}), SHUTTLI_DATA_DIR=str(profile))
    adb = [config.get("adb", "adb"), "-s", serial]
    binary = config["binary"]
    xclip = config.get("xclip", "xclip")

    def command(*words):
        result = subprocess.check_output([binary, *words, "--json"], env=env, timeout=20)
        answer = json.loads(result)
        assert answer["type"] != "error", answer
        return answer

    status = command("status")["status"]
    assert status["device"] == desktop_id
    assert status["settings"]["send"] and status["settings"]["receive"]
    assert status["settings"]["history"] == "content"
    assert status["settings"]["automatic"]
    subprocess.run([*adb, "shell", "am", "start", "-n", "org.katinbroek.shuttli/.MainActivity"],
                   check=True, stdout=subprocess.DEVNULL, timeout=10)
    command("refresh")
    deadline = time.monotonic() + 60
    while True:
        devices = command("devices")["devices"]
        if any(p["id"] == phone_id and p["online"] for p in devices):
            break
        assert time.monotonic() < deadline, "Foreground app was not discovered"
        time.sleep(1)
    assert all(not status["settings"]["peers"].get(p["id"], {}).get("send", False)
               for p in devices if p["id"] != phone_id), "Other outgoing peers must be disabled"

    def read_clipboard(mime):
        return subprocess.check_output([xclip, "-selection", "clipboard", "-o", "-t", mime],
                                       env=env, timeout=5, stderr=subprocess.DEVNULL)

    backup = None
    for mime in ("image/png", "UTF8_STRING"):
        try:
            backup = (mime, read_clipboard(mime))
            break
        except subprocess.CalledProcessError:
            pass
    assert backup is not None, "Cannot safely preserve the desktop clipboard"

    def write_clipboard(data, mime="UTF8_STRING"):
        # xclip forks a selection owner. Its inherited pipes must not hold the coordinator open.
        subprocess.run([xclip, "-selection", "clipboard", "-i", "-t", mime], input=data,
                       env=env, check=True, timeout=5, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def fixture(text):
        command("history", "clear")
        write_clipboard(text.encode())

    messages = queue.Queue()
    subprocess.run([*adb, "logcat", "-c"], check=True)
    logger = subprocess.Popen([*adb, "logcat", "-v", "brief", "-s", "ShuttliTailnetTest:I"],
                              stdout=subprocess.PIPE, text=True)

    def collect():
        with (output / "events.log").open("w") as log:
            for line in logger.stdout:
                log.write(line)
                log.flush()
                messages.put(line)

    collector = threading.Thread(target=collect, daemon=True)
    collector.start()
    options = ["-e", "tailnetPeer", desktop_id, "-e", "tailnetToken", token]
    if second:
        options += ["-e", "tailnetSecondPeer", second["id"]]
    process = None
    original_send = status["settings"]["peers"].get(phone_id, {}).get("send", False)
    phone_digest = None
    desktop_digest = None
    try:
        command("peer", phone_id, "send", "on")
        with (output / "instrumentation.log").open("w") as log:
            process = subprocess.Popen([*adb, "shell", "am", "instrument", "-w",
                *(["-e", "coverage", "true"] if config.get("instrumentation_coverage") else []), "-e", "class",
                "org.katinbroek.shuttli.TailnetInstrumentedTest", *options,
                "org.katinbroek.shuttli.test/androidx.test.runner.AndroidJUnitRunner"], stdout=log,
                stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 600
            while process.poll() is None and time.monotonic() < deadline:
                try:
                    line = messages.get(timeout=1)
                except queue.Empty:
                    continue
                if "PHONE_PNG_SHA256=" in line:
                    phone_digest = line.split("PHONE_PNG_SHA256=")[1].strip()
                if "DESKTOP_PNG_SHA256=" in line:
                    assert line.split("DESKTOP_PNG_SHA256=")[1].strip() == desktop_digest
                match = re.search(r"CHECKPOINT=([a-z-]+)", line)
                if not match:
                    continue
                step = match[1]
                print(step, flush=True)
                if step == "desktop-text":
                    fixture("desktop tailnet text " + token)
                elif step == "desktop-incremental":
                    write_clipboard(("desktop incremental text " + token).encode())
                    rows = command("history")["entries"]
                    assert not any(row["direction"] == "send" and row["peer"] == phone_id
                                   for row in rows), "Pull-only history must not create live delivery rows"
                elif step == "phone-text-applied":
                    assert read_clipboard("UTF8_STRING") == ("phone tailnet text " + token).encode()
                elif step == "phone-image-applied":
                    assert hashlib.sha256(read_clipboard("image/png")).hexdigest() == phone_digest
                elif step == "background":
                    with socket.socket() as probe:
                        probe.settimeout(3)
                        assert probe.connect_ex((str(phone_ip), 45987)) != 0, "Background listener is open"
                    fixture("desktop offline text " + token)
                elif step == "desktop-image":
                    from PIL import Image
                    image = io.BytesIO()
                    Image.new("RGBA", (3, 2), (211, 73, 83, 255)).save(image, format="PNG")
                    data = image.getvalue()
                    desktop_digest = hashlib.sha256(data).hexdigest()
                    command("history", "clear")
                    write_clipboard(data, "image/png")
                elif step == "receive-disabled":
                    fixture("desktop denied text " + token)
                elif step == "desktop-send-disabled":
                    command("peer", phone_id, "send", "off")
                    fixture("desktop consent text " + token)
                elif step == "desktop-send-reenabled":
                    command("peer", phone_id, "send", "on")
                elif step == "multiple-sources":
                    assert second, "Second source configuration missing"
                    subprocess.run(second["prepare_command"], check=True, timeout=30,
                                   env=dict(env, SHUTTLI_TEST_TOKEN=token))
                elif step not in ("receive-reenabled", "multiple-sources-merged"):
                    raise AssertionError("Unknown checkpoint: " + step)
                # Only the separately installed test APK uses this private acknowledgement file.
                ack = f"run-as org.katinbroek.shuttli sh -c 'echo ok > files/tailnet-{token}-{step}'"
                subprocess.run([*adb, "shell", ack], check=True, timeout=5)
            assert process.poll() is not None, "Instrumentation timed out"
        result = (output / "instrumentation.log").read_text()
        assert "OK (1 test)" in result, result
        print("Real Tailscale clipboard/history/consent test passed")
    finally:
        if process is not None and process.poll() is None:
            subprocess.run([*adb, "shell", "am", "force-stop", "org.katinbroek.shuttli"], timeout=10)
            process.wait(timeout=10)
        logger.terminate()
        logger.wait(timeout=5)
        collector.join(timeout=5)
        try:
            command("peer", phone_id, "send", "on" if original_send else "off")
        finally:
            write_clipboard(backup[1], backup[0])


if __name__ == "__main__":
    main()
