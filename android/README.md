# smert words: Android lock-screen wallpaper

A live wallpaper that shows one of smert's two daily words with its full
definition, from `/api/wotd`. The language alternates each time the screen
turns on. It is meant for the lock screen only (set a normal wallpaper on the
home screen).

- Fetches when the screen turns on (at most every 15 minutes, so rerolls show
  up), and just after the 05:00 rollover. The last response is kept for when
  the phone is offline.
- Text starts just below the lock-screen clock and stays above the bottom
  shortcuts (`AREA_TOP` / `AREA_BOTTOM` in `WordsWallpaperService.kt`, measured
  for the one-line clock style), shrinking to fit long definitions.
- Font: Baskervville (SIL Open Font License, see `licenses/`).

## Building

Needs JDK 17 and the Android SDK (`~/Library/Android/sdk`, with
`sdk.dir` in `local.properties`):

```bash
cd android
JAVA_HOME=/opt/homebrew/opt/openjdk@17 ./gradlew assembleDebug
```

The app file is `app/build/outputs/apk/debug/app-debug.apk`.

## Installing on the phone

With wireless debugging on (Developer options → Wireless debugging) and the
phone paired once with `adb pair`:

```bash
adb connect PHONE_IP:PORT
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Opening "smert words" on the phone opens the live wallpaper preview; choose
**Lock screen** when setting it.

## smert admin

The same app adds a second icon, **smert admin** (the fleur-de-lis), which
opens the server's `/admin` page full screen. To launch it from the lock
screen: Settings → Lock screen → Shortcuts, and pick it for the left or right
shortcut. The phone asks to be unlocked before it opens.

Links to other sites (Bible Gateway) open in the browser.

## When the server gets a domain

`SERVER_URL` (in `Server.kt`) points at the sslip.io address over plain HTTP,
which `res/xml/network_security_config.xml` allows for that host only. With a
domain and HTTPS, update `SERVER_URL` and delete that exception.
