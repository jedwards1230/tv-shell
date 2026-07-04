# Host Setup & Pairing

`game-shell-host` is a small cross-platform sidecar you run **on the gaming PC**
(the Steam machine that Sunshine streams from). The game-shell TV client's daemon
talks to it over HTTP to list installed Steam games, show their art on the home
screen, and launch a game into Big Picture before Moonlight streams it.

```
game-client (TV)                          gaming PC (Steam host)
  game-shell-input daemon ──HTTP :47995──▶ game-shell-host
    [steam] url   (config.toml)              GAME_SHELL_HOST_TOKEN  (env)
    [steam] token (bearer, must match the host token)
```

It never touches Sunshine config, so other Moonlight clients are unaffected.

> ⚠️ **Security — trusted LANs only.** The daemon talks to the host over
> **unencrypted HTTP** with a **static bearer token**. That's safe only on an
> isolated, trusted LAN where you control every host — a passive observer (ARP
> spoof, rogue DHCP, WiFi sniffer) on the same segment can capture the token.
> Do not expose the host to guest WiFi, untrusted networks, or the Internet
> without putting it behind TLS or a VPN (e.g. Tailscale). The token is a shared
> secret stored in plaintext on both machines and is not rotated automatically —
> rotate it by hand if it leaks or when moving to a different network.

## Endpoints

| Method | Path          | Auth        | Purpose |
|--------|---------------|-------------|---------|
| GET    | `/library`    | Bearer      | Enumerate installed Steam games (VDF/ACF) |
| POST   | `/launch`     | Bearer      | Navigate Big Picture to a game's page (user presses Play) |
| GET    | `/status`     | Bearer      | `{ version, running_appid, streaming }` |
| GET    | `/art/{appid}`| **public**  | Local cover art (no bearer — QML `Image.source` can't send one; art isn't sensitive) |

## Environment

| Var | Default | Meaning |
|-----|---------|---------|
| `GAME_SHELL_HOST_TOKEN` | random (logged at startup) | Bearer token. **Set it** to a stable value so the daemon can be paired. |
| `GAME_SHELL_HOST_PORT`  | `47995` | Listen port (chosen outside Sunshine's 47984–47990 range). |
| `GAME_SHELL_HOST_BIND`  | `0.0.0.0` | Listen address (all LAN interfaces). |

Generate a token once and reuse it on both ends:

```bash
openssl rand -hex 16
```

---

## Install path A — Ansible-managed (homelab)

On the gaming host the service is a managed `systemd --user` unit via the `desktop-common`
role. You don't run anything by hand — set the flags and apply:

```yaml
# host_vars/gaming-host.yaml
game_shell_host_enabled: true
game_shell_host_install_method: fetch          # download the released binary
game_shell_host_version: "0.1.0"               # the host-v<version> release tag
game_shell_host_binary_sha256: "<from the release checksums.txt>"
game_shell_host_token: !vault | ...            # vault-encrypted; reuse the daemon's token
```

```bash
ansible-playbook playbooks/site-desktop.yml --tags game-shell-host,firewall
```

The role installs the binary to `/usr/local/bin/game-shell-host`, writes a `0600`
env file, enables linger so the unit survives logout/reboot, and opens the LAN
firewall to port 47995. See the role for the full variable list.

## Install path B — manual (any gaming PC)

1. **Download** the binary for your OS from the latest `host-v*`
   [release](https://github.com/jedwards1230/game-shell/releases):

   | OS | Asset |
   |----|-------|
   | Linux | `game-shell-host-x86_64-unknown-linux-musl` |
   | macOS (Apple Silicon) | `game-shell-host-aarch64-apple-darwin` |
   | macOS (Intel) | `game-shell-host-x86_64-apple-darwin` |
   | Windows | `game-shell-host-x86_64-pc-windows-msvc.exe` |

   Verify it against the release `checksums.txt`, then install it
   (e.g. `install -m755 game-shell-host-* /usr/local/bin/game-shell-host`).

2. **Run it** with a stable token. Quick smoke test:

   ```bash
   GAME_SHELL_HOST_TOKEN=<token> game-shell-host
   # → game-shell-host listening on 0.0.0.0:47995
   ```

   To keep it running, install it as a service. On Linux a user unit bound to the
   graphical session works (Steam must reach the live desktop session to launch a
   game) — mirror `roles/desktop-common/templates/game-shell-host.service.j2` from
   the homelab-ansible repo. On Windows use Task Scheduler (at-logon); on macOS a
   launchd LaunchAgent.

3. **Open the firewall** to the LAN so the TV box can reach it:

   ```bash
   # Linux (firewalld)
   firewall-cmd --permanent --add-rich-rule='rule family="ipv4" \
     source address="192.0.2.0/24" port port="47995" protocol="tcp" accept'
   firewall-cmd --reload
   ```

   The `/24` source opens the port to the whole subnet — fine if every host on it
   is trusted. To tighten it, scope the rule to the TV client's IP only (e.g.
   `source address="192.0.2.50/32"`) so a guest or compromised device on the LAN
   can't reach the control surface.

### Windows

1. **Download** `game-shell-host-x86_64-pc-windows-msvc.exe` from the latest
   `host-v*` [release](https://github.com/jedwards1230/game-shell/releases) and
   verify it against `checksums.txt`.

2. **Set env vars** for the same user session Steam runs in: `GAME_SHELL_HOST_TOKEN`
   (the **same** token used on the Linux boot, see the dual-boot note below),
   optionally `STEAM_PATH` (only if Steam isn't at the default
   `C:\Program Files (x86)\Steam`) and `GAME_SHELL_HOST_PORT`.

3. **Open the firewall** (LAN-scoped):

   ```powershell
   New-NetFirewallRule -DisplayName "game-shell-host" -Direction Inbound -Action Allow `
     -Protocol TCP -LocalPort 47995 -RemoteAddress 192.0.2.0/24
   ```

4. **Auto-start at logon** via Task Scheduler (no extra deps). Set the token as a
   persistent user env var first (Task Scheduler inherits the user environment),
   then register the task:

   ```powershell
   setx GAME_SHELL_HOST_TOKEN <token>
   schtasks /Create /TN "game-shell-host" /TR "\"C:\path\to\game-shell-host.exe\"" `
     /SC ONLOGON /RL LIMITED /F
   ```

   Steam must be running in the same interactive session for launches to work.

> **Dual-boot note.** desktop-2 (Windows) and the CachyOS boot are the **same
> machine, same LAN IP**. Reuse the same `GAME_SHELL_HOST_TOKEN` on both OSes so
> the TV daemon's `[steam]` config (`url` + `token`) never has to change —
> whichever OS is currently booted answers on the same IP:port with zero
> client-side changes.

> **Big Picture nav timing on Windows**: the daemon fires the `steam://nav/...`
> URL immediately with no "is Big Picture up yet?" wait (unlike Linux) — see
> `host/src/launch.rs`'s `wait_for_bigpicture` doc comment for why.

---

## Pair the daemon (the TV box)

Point the daemon at the host and give it the **same token**:

```toml
# ~/.config/game-shell/config.toml on the game-client
[steam]
url = "http://<host-ip>:47995"
# Either inline …
token = "<same token as GAME_SHELL_HOST_TOKEN>"
# … or, preferred, a 0600 file: token_file = "~/.config/game-shell/steam-token"
```

Restart the daemon to pick it up. Under config management this is typically
wired by your deployment role — set the Steam URL and token via your own host
variables and render `config.toml` from them.

Keep the token private: it's the same secret on both machines in plaintext. Prefer
`token_file` and `chmod 0600` it (the host's own env file too), keep it out of
shell history, and vault it in any config repo. It isn't rotated by default —
rotate it on both ends together if it leaks.

## Verify

```bash
# Authenticated — lists games
curl -H "Authorization: Bearer <token>" http://<host-ip>:47995/library

# Public — cover art for an appid (e.g. 1245620)
curl -o /tmp/art.jpg http://<host-ip>:47995/art/1245620
```

A populated `/library` and a Steam row on the TV home screen means pairing
worked. If `/library` is empty, confirm Steam is installed and has at least one
game; if it 401s, the daemon's token doesn't match the host's.
