# smert: macOS wallpaper and lock screen

A background agent that shows smert's daily artwork, from `/api/artwork`, as
the desktop wallpaper of every screen, scaled to meet the nearer edges of the
screen with the rest filled by a blur of its edges' colours, and draws its daily quote, from `/api/quote`, on the lock screen, as
the display page shows it.

- macOS has no lock screen picture of its own: it shows the wallpaper. So the
  agent puts the quote up when the screen locks and the artwork back when it
  unlocks, which macOS shows as a short crossfade. The login window after a
  restart shows whichever was up last.
- Until there is an artwork (or while its image downloads) the desktop shows
  the quote too.
- Keeps `/api/events` open and refetches the moment the quote or artwork is
  rerolled, added or deleted (well under a second). The 05:00 rollover sends no event,
  so it also refetches just after it, and hourly as a safety net.
- If the event stream drops (redeploy, sleep, network change) it reconnects
  after 1 s, doubling up to a minute, and at once on wake; each reconnect
  refetches, to catch what was missed. The server sends a keep-alive every
  20 s, so 60 s of silence counts as a dead stream.
- The last responses, and the artwork's image, are kept for when the Mac is
  offline.
- macOS only sets the wallpaper of the current Space, so the agent sets it
  again whenever you switch to a Space that doesn't show it yet.
- Long quotes shrink to fit between the menu bar and the Dock.
- Font: Baskervville (SIL Open Font License, see `../android/licenses/`),
  copied from the Android app; macOS's Baskerville if it is missing.

## Installing

Needs the Xcode command line tools (`swiftc`):

```bash
macos/install.sh
```

This builds `SmertWallpaper.swift` into
`~/Library/Application Support/smert-wallpaper/` and registers it with launchd
(`~/Library/LaunchAgents/com.leopoldarkham.smert.wallpaper.plist`), so it runs
at login and restarts if it exits. Run it again after changing the source.

To remove it (the last wallpaper stays until you pick another):

```bash
macos/install.sh uninstall
```

## Testing against a local server

```bash
SMERT_URL=http://127.0.0.1:1212 "$HOME/Library/Application Support/smert-wallpaper/SmertWallpaper" --once
```

`--once` fetches, sets the wallpaper and exits; leave it off to also listen
for rerolls. To watch the events themselves:

```bash
curl -N http://127.0.0.1:1212/api/events
```
