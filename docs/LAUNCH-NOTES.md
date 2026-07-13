# Launch post material

Raw material worth covering in the Exodium announcement. Not a draft - a
collection of the stories that make the launcher worth writing about.

## The soundtrack story (lead candidate)

Exodium plays DOS games **as they were composed**. Sierra hired real
musicians - Space Quest III's opening theme is by Bob Siebenberg, Supertramp's
drummer - and they composed on a Roland MT-32, a $550 hardware synthesizer.
The SoundBlaster versions most people remember are lossy FM-chip downports
made after the fact.

What Exodium does automatically, per game:

- Downloads the Roland MT-32/CM32L ROMs and the SC-55 SoundCanvas soundfont
  from the eXoDOS collection itself (one-time, only when a MIDI game is
  installed).
- Translates eXoDOS's DOSBox-ECE audio configuration into DOSBox Staging's
  format at launch - ~1,500 game configs carry ECE-specific keys that would
  otherwise be silently ignored.
- Runs MT-32 music and SoundBlaster digital effects **simultaneously**, the
  way the eXo team configured each game - authentic Roland score plus sampled
  speech/effects.

Result: ~700 MT-32 games and ~2,200 General MIDI games play their authored
soundtracks on Linux, macOS, and Windows.

## Other angles

- **One tiny install, 7,600+ games on demand**: ~4 MB of bundled metadata
  replaces the 4.9 GB LaunchBox archive; individual games stream from the
  eXoDOS torrents. Seeding is on by default (disclosed at setup, toggleable) -
  every player strengthens the preservation swarm.
- **eXo's configs are law**: per-game dosbox.conf files run as authored -
  machine type, cycles, CD mounts, multi-step autoexecs. Language-pack games
  run the same configs via an overlay mount instead of guessed launch
  commands.
- **Language packs as first-class citizens**: German, Spanish, and Polish
  variants merged into one card per game with per-language install state.
- **Saves survive**: uninstall backs up the whole game dir; reinstall
  restores it.
- **Approved by the eXoDOS project.**

## Honest caveats to state upfront

- All games run under DOSBox Staging. A handful of titles tuned for
  DOSBox-ECE / DOSBox-X specials (3dfx Voodoo passthrough, GunStick,
  ~19 special builds) may differ from the original Windows eXoDOS setup.
- macOS builds are unsigned for now (documented one-line `xattr` fix).
