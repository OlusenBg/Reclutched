# Reclutched
A stats and clipping app for other fellow linux users !

## Desktop app (work in progress)

`reclutched-ui` is a GTK4 / libadwaita app. For now it has the first-launch setup (username, display mode,
what to track); the match history comes next. It needs GTK 4 and libadwaita installed.

```bash
sudo pacman -S gtk4 libadwaita          # Arch
cargo run -p reclutched-ui --release
```

Your choices are saved in `~/.config/reclutched/settings.json`, the same file `reclutched config` uses.
`reclutched-ui --reset-setup` runs the setup again.
