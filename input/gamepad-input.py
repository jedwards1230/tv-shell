#!/usr/bin/env python3
"""Game Shell input daemon.

Grabs a gamepad exclusively via EVIOCGRAB and emits keyboard events via uinput.
Listens on a unix socket for grab/release/subscribe commands from the shell.
"""

import asyncio
import logging
import os
import signal
import sys
from pathlib import Path

import evdev
from evdev import InputDevice, UInput, ecodes, categorize

log = logging.getLogger("game-shell-input")

VENDOR_ID = int(os.environ.get("GAMEPAD_VENDOR", "0x045e"), 0)
PRODUCT_ID = int(os.environ.get("GAMEPAD_PRODUCT", "0x028e"), 0)

SOCK_PATH = os.environ.get(
    "GAME_SHELL_SOCK",
    f"/run/user/{os.getuid()}/game-shell-input.sock",
)

BUTTON_MAP = {
    ecodes.BTN_SOUTH: ecodes.KEY_ENTER,   # A → Enter
    ecodes.BTN_EAST: ecodes.KEY_ESC,      # B → Escape
    ecodes.BTN_NORTH: ecodes.KEY_TAB,     # Y → Tab
    ecodes.BTN_START: ecodes.KEY_ENTER,   # Start → Enter
}

COMBO_KEYS = {ecodes.BTN_MODE, ecodes.BTN_EAST}
COMBO_HOLD_SECS = 3.0


def find_gamepad() -> InputDevice | None:
    for path in sorted(evdev.list_devices()):
        dev = InputDevice(path)
        if dev.info.vendor == VENDOR_ID and dev.info.product == PRODUCT_ID:
            caps = dev.capabilities(verbose=False)
            if ecodes.EV_KEY in caps and ecodes.BTN_SOUTH in caps[ecodes.EV_KEY]:
                return dev
    return None


class InputDaemon:
    def __init__(self):
        self.gamepad: InputDevice | None = None
        self.uinput: UInput | None = None
        self.grabbed = False
        self.subscribers: list[asyncio.StreamWriter] = []
        self.held_keys: set[int] = set()
        self.combo_task: asyncio.Task | None = None
        self.running = True

    async def start(self):
        self.uinput = UInput(
            {ecodes.EV_KEY: list(BUTTON_MAP.values()) + [
                ecodes.KEY_UP, ecodes.KEY_DOWN, ecodes.KEY_LEFT, ecodes.KEY_RIGHT,
            ]},
            name="game-shell-virtual-kb",
        )
        log.info("uinput device created: %s", self.uinput.device.path)

        asyncio.create_task(self._serve_socket())
        asyncio.create_task(self._device_loop())

    async def _device_loop(self):
        """Find gamepad, grab it, read events. Reconnect on disconnect."""
        while self.running:
            self.gamepad = find_gamepad()
            if not self.gamepad:
                log.info("Gamepad not found (vendor=%04x product=%04x), retrying...",
                         VENDOR_ID, PRODUCT_ID)
                await asyncio.sleep(2)
                continue

            log.info("Found gamepad: %s at %s", self.gamepad.name, self.gamepad.path)
            await self._grab()

            try:
                async for event in self.gamepad.async_read_loop():
                    if not self.grabbed:
                        continue
                    await self._handle_event(event)
            except OSError:
                log.warning("Gamepad disconnected, will reconnect...")
                self.gamepad = None
                self.grabbed = False
                await asyncio.sleep(1)

    async def _handle_event(self, event):
        if event.type == ecodes.EV_KEY:
            key_event = categorize(event)

            # Track held state for combo detection
            if key_event.keystate == 1:  # down
                self.held_keys.add(event.code)
                self._check_combo_start()
            elif key_event.keystate == 0:  # up
                self.held_keys.discard(event.code)
                self._cancel_combo()

            # Map to keyboard
            mapped = BUTTON_MAP.get(event.code)
            if mapped and self.uinput:
                self.uinput.write(ecodes.EV_KEY, mapped, key_event.keystate)
                self.uinput.syn()

        elif event.type == ecodes.EV_ABS:
            # D-pad hat → arrow keys
            if event.code == ecodes.ABS_HAT0X:
                if event.value == -1:
                    self._emit_key(ecodes.KEY_LEFT, 1)
                elif event.value == 1:
                    self._emit_key(ecodes.KEY_RIGHT, 1)
                else:
                    self._emit_key(ecodes.KEY_LEFT, 0)
                    self._emit_key(ecodes.KEY_RIGHT, 0)
            elif event.code == ecodes.ABS_HAT0Y:
                if event.value == -1:
                    self._emit_key(ecodes.KEY_UP, 1)
                elif event.value == 1:
                    self._emit_key(ecodes.KEY_DOWN, 1)
                else:
                    self._emit_key(ecodes.KEY_UP, 0)
                    self._emit_key(ecodes.KEY_DOWN, 0)

    def _emit_key(self, key, value):
        if self.uinput:
            self.uinput.write(ecodes.EV_KEY, key, value)
            self.uinput.syn()

    def _check_combo_start(self):
        if COMBO_KEYS.issubset(self.held_keys) and self.combo_task is None:
            self.combo_task = asyncio.create_task(self._combo_timer())

    def _cancel_combo(self):
        if self.combo_task and not COMBO_KEYS.issubset(self.held_keys):
            self.combo_task.cancel()
            self.combo_task = None

    async def _combo_timer(self):
        try:
            await asyncio.sleep(COMBO_HOLD_SECS)
            if COMBO_KEYS.issubset(self.held_keys):
                log.info("End-session combo detected")
                await self._notify_subscribers("combo:end-session")
        except asyncio.CancelledError:
            pass
        finally:
            self.combo_task = None

    async def _grab(self):
        if self.gamepad and not self.grabbed:
            try:
                self.gamepad.grab()
                self.grabbed = True
                log.info("Grabbed gamepad exclusively")
            except OSError as e:
                log.error("Failed to grab gamepad: %s", e)

    async def _ungrab(self):
        if self.gamepad and self.grabbed:
            try:
                self.gamepad.ungrab()
                self.grabbed = False
                log.info("Released gamepad grab")
            except OSError as e:
                log.error("Failed to ungrab gamepad: %s", e)

    async def _notify_subscribers(self, message: str):
        dead = []
        for writer in self.subscribers:
            try:
                writer.write((message + "\n").encode())
                await writer.drain()
            except (ConnectionError, BrokenPipeError):
                dead.append(writer)
        for w in dead:
            self.subscribers.remove(w)

    async def _serve_socket(self):
        sock_path = Path(SOCK_PATH)
        sock_path.unlink(missing_ok=True)

        server = await asyncio.start_unix_server(self._handle_client, path=str(sock_path))
        os.chmod(str(sock_path), 0o600)
        log.info("Listening on %s", sock_path)

        async with server:
            await server.serve_forever()

    async def _handle_client(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
        try:
            while True:
                data = await reader.readline()
                if not data:
                    break
                cmd = data.decode().strip()

                if cmd == "grab":
                    await self._grab()
                    writer.write(b"ok\n")
                elif cmd == "release":
                    await self._ungrab()
                    writer.write(b"ok\n")
                elif cmd == "status":
                    status = "connected" if self.gamepad else "disconnected"
                    grabbed = "grabbed" if self.grabbed else "released"
                    writer.write(f"{status}:{grabbed}\n".encode())
                elif cmd == "subscribe":
                    self.subscribers.append(writer)
                    writer.write(b"subscribed\n")
                    await writer.drain()
                    # Keep connection open for events
                    await reader.read()
                    return
                else:
                    writer.write(b"unknown\n")

                await writer.drain()
        except (ConnectionError, BrokenPipeError):
            pass
        finally:
            if writer in self.subscribers:
                self.subscribers.remove(writer)
            writer.close()


async def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(name)s: %(message)s",
        stream=sys.stdout,
    )

    daemon = InputDaemon()

    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, lambda: setattr(daemon, "running", False))

    await daemon.start()

    while daemon.running:
        await asyncio.sleep(1)

    if daemon.uinput:
        daemon.uinput.close()
    log.info("Shutting down")


if __name__ == "__main__":
    asyncio.run(main())
