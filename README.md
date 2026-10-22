<p align="center">
  <img src="branding/logo/banner.png" alt="Reclutched" width="720">
</p>

<p align="center">
  A calm, local stats tracker for Rocket League, built for Linux (works with Proton / Wine).<br>
  Clipping and an in-game overlay are planned.
</p>

---

## What it does

- **Tracks your matches by itself.** Open the app, start Rocket League, and every match is saved when it ends.
- **Match history and details.** Result, score, mode and time, then Overview, Players, Goals and Movement
  (possession, boost habits, speed, touches, goals with your boost at that moment).
- **Live window for a second screen.** Choose *2 Screens* or *Both* in the setup (or settings) and a window opens
  by itself when a match starts: score and clock, possession for both teams, your boost and speed, a live
  scoreboard and the latest goals, saves and demolitions. Close it and it stays closed for that match.
- **In-game overlay.** Choose *Overlay* or *Both*: a small "Tracking" light sits in the corner of the game (red
  "Stats API off" when the game is not sending stats), and while you hold a key (Tab by default) or a controller
  button a panel shows
  possession on top, your habits as bars on the left (each with a tick for your own average from past matches)
  and the players on the right. See [the overlay](#the-in-game-overlay) below.
- **Everything is a switch.** Each thing we collect can be turned on or off in the settings.
- **Light on disk.** Only averages and small event rows are stored, never the raw 10-per-second data.

## Private by design

- Uses only the game's **official local Stats API**. No injection, no hooks, nothing that touches the game.
- Everything stays on your computer in a local SQLite file. Nothing is uploaded.
- The overlay key and controller button are read from your input devices only while the overlay is on; every
  other key and button is dropped on the spot (nothing is stored, logged or sent), and the game still gets them.
- Your settings live in `~/.config/reclutched/settings.json`, your matches in
  `~/.local/share/reclutched/matches.db`.

## Install (Arch Linux)

You need GTK 4 and libadwaita, plus a Rust toolchain.

```bash
sudo pacman -S --needed gtk4 libadwaita gtk4-layer-shell base-devel rust git
git clone https://github.com/OlusenBg/Reclutched.git
cd Reclutched
cargo install --path crates/reclutched-ui --locked
cargo install --path crates/reclutched-cli --locked
reclutched-ui
```

The first build takes a few minutes. Cargo installs into `~/.cargo/bin`, so make sure that folder is in your
`PATH` (fish: `fish_add_path ~/.cargo/bin`).

To get it in your app launcher (menu entry and icon), run this once from the repository folder:

```bash
packaging/install-desktop.sh            # undo with: packaging/install-desktop.sh --remove
```

The first launch walks you through a short setup. `reclutched-ui --reset-setup` runs it again.

### Arch package (PKGBUILD)

`packaging/aur/PKGBUILD` builds and installs everything properly (both programs, the launcher entry and the
icon) as a package, so pacman can update or remove it:

```bash
cd packaging/aur
makepkg -si
```

It is not published on the AUR yet.

### The in-game overlay

The overlay is drawn on a Wayland layer-shell surface above the game, and mouse clicks pass straight through
it. It needs:

- a Wayland desktop with layer-shell (Hyprland, Sway, KDE Plasma; not GNOME) and the package `gtk4-layer-shell`;
- the game in **borderless windowed** mode (a fullscreen game hides every overlay);
- to show only on the game window, Hyprland for now (we read its focus events). On other desktops the overlay
  shows while the game is connected;
- permission to read your keyboard and controller for the hold key or button (Wayland apps cannot see them while
  the game has the focus). Add yourself to the `input` group once, then log out and in:

  ```bash
  sudo usermod -aG input $USER
  ```

Set the key and the button in **Settings → Overlay**: press **Change**, then press the key (or the button on your
controller). Tab is the default key, the game's own scoreboard key; the controller button is not set by default.
You can have both, either one shows the overlay. The D-pad works as a button too. If the game window
is not recognised, add a word from its class or title to `game_window_hints` in
`~/.config/reclutched/settings.json` (`hyprctl activewindow` shows them).

### Solid window on Hyprland

Hyprland draws every window at `decoration:active_opacity` (HyDE sets 0.9, and 0.75 for inactive windows), so
the app can look see-through and washed out. An app cannot opt out of this itself. Two ways to fix it:

**1. Make all windows solid.** This is the quick one, and it worked on Hyprland 0.56 with the Lua config. It may be
reset when Hyprland reloads its config, so put the same two values in your config to keep them:

```bash
hyprctl eval 'hl.config({ decoration = { active_opacity = 1.0, inactive_opacity = 1.0 } })'
```

On older versions: `hyprctl keyword decoration:active_opacity 1.0` (and the same for `inactive_opacity`).

**2. Make only Reclutched solid** with a window rule, once. On Hyprland 0.55+ with the Lua config
(`~/.config/hypr/hyprland.lua`):

```lua
hl.window_rule({
  name = "reclutched_opaque",
  match = { class = "^dev\\.reclutched\\.Reclutched$" },
  opaque = true,
})
```

On older versions with `hyprland.conf` (the exact keyword changed across releases, check the wiki for yours):

```ini
windowrule = opaque, class:^(dev.reclutched.Reclutched)$
```

Settings shows the Lua line with a copy button when it runs under Hyprland. The in-game overlay is transparent
on purpose and is not affected. The app also stays dark and solid under themed `~/.config/gtk-4.0/gtk.css` files.

### Turn on the Stats API

Rocket League only shares your stats when its Stats API is on. In the app, open **Settings** and use
**Find** (or **Choose…**) and **Turn on** under *Rocket League*. It only changes the `PacketSendRate` line of
the game's config file and keeps a backup. Restart the game afterwards.

### Try it without playing

```bash
cargo run -p reclutched-core --example seed_demo -- /tmp/demo.db
reclutched-ui --db /tmp/demo.db
```

## Command line

The same engine is available without a window:

```bash
reclutched track                  # save matches while you play
reclutched history                # list saved matches (--detail, --match ID)
reclutched config list            # show what is collected
reclutched config enable|disable  # switch collectors on or off
reclutched dump                   # print the raw Stats API messages
```

Run only one of the app or `reclutched track` at a time.

## Roadmap

| Stage | |
|---|---|
| 1. Stats tracking | Done |
| 2. Desktop app (UI/UX) | In progress: setup, history, details and settings work; testing and first release next |
| 3. Capturing and clipping | Planned (will use an existing recorder) |
| 4. Overlay | First version (needs testing on more desktops) |

## Project layout

| Crate | What it is |
|---|---|
| `reclutched-core` | Stats API parsing, the match tracker, SQLite storage, settings |
| `reclutched-daemon` | Connects to the game and saves matches |
| `reclutched-cli` | The `reclutched` command |
| `reclutched-ui` | The GTK4 / libadwaita app, the live window and the overlay |

The logo and its alternatives are in [`branding/`](branding/).

## Licence

Reclutched is free software under the [GNU GPL v3 or later](LICENSE). You can use and change it, and if you
share a modified version it has to stay open source under the same licence.
