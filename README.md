# Reclutched
A stats and clipping app for other fellow linux users !

## Desktop app

`reclutched-ui` is a GTK4 / libadwaita app: your match history on the left, the details of each match on
the right (Overview, Players, Goals, Movement), and a settings page for everything you can switch on or off.
On the first launch it walks you through a short setup. It needs GTK 4 and libadwaita installed.

```bash
sudo pacman -S gtk4 libadwaita          # Arch
cargo run -p reclutched-ui --release
```

The app shows what `reclutched track` has saved, so keep tracking running while you play (a terminal is fine
for now). Your choices are saved in `~/.config/reclutched/settings.json`, the same file `reclutched config`
uses. `reclutched-ui --reset-setup` runs the setup again.

To look at the app without playing, fill a scratch database with made-up matches:

```bash
cargo run -p reclutched-core --example seed_demo -- /tmp/demo.db
cargo run -p reclutched-ui --release -- --db /tmp/demo.db
```
