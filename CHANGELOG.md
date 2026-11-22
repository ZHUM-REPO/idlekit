# Changelog

The section for the version being released becomes the text of its release page.

## 0.9

Fixes for two faults that 0.8 brought to the Logs page, and two precautions.

- **The Logs page follows the newest line again.** Since 0.8 it stopped following once it held 1,500 lines: new lines arrived below the visible part.
- **A long newest line is shown whole.** Since 0.8 a new line long enough to wrap showed only its first row until the next line arrived.
- **Log lines reach the window even when it draws nothing.** New lines are added when the window draws a frame. If it draws none, as a hidden window may not, a timer now adds them within half a second, and no more lines wait than the page keeps. `log.txt` was never affected.
- **Each idling game's helper checks in with Steam four times a second.** 0.8 lowered this from ten times to once, which is slower than Valve advises. Nothing is known to have gone wrong with once; four keeps most of the saving.

## 0.8

Speed work from the performance review. Nothing looks or behaves differently.

- **The Library redraws only what changed.** Selecting a game, typing in the filter or changing a list used to redraw every tile on the page; now only the tiles whose game changed are redrawn. With 150 tiles shown, selecting a game went from 14–19 ms of work to 1–5 ms.
- **Bulk actions are one step.** Adding many selected games to auto-idle is one command with one save, not one of each per game.
- **A smaller, more tightly built program.** The release is now optimized across the whole program. On the Linux test build the app shrank by a quarter.
- **Less work in the background.** The top bar and the Idling, Card farming and Achievements pages no longer redraw every second when nothing on them is counting. Each idling game's helper checks in with Steam once a second, not ten times. The Logs page draws a new line without redrawing the others.
- **Small savings.** Text patterns used to read Steam's pages are built once and reused, and reading one setting no longer copies all of them.

## 0.7

- **The window has its own title bar.** The separate Windows bar above the app is gone. Minimize, maximize and close now sit at the right end of IdleKit's top bar, and the window is moved by dragging any empty part of that bar or the logo beside it. A double click on the bar maximizes or restores the window.
- Closing with the new button works as before: it asks, goes to the tray or quits, whichever is set in Settings.
- The window is still resized by its edges, and Alt+F4 and the Windows key with the arrow keys work as they did. The layout menu that Windows 11 shows when the mouse rests on a maximize button does not appear on the app's own button; dragging the window to a screen edge still snaps it.

## 0.6

- **Pictures no longer take over the connection.** Opening the Library used to start a download for every cover near the screen at once, dozens at a time, which could slow the whole network on a slow line. Now at most three pictures are fetched at a time: the tiles on screen first, then the ones just below, and nothing for tiles far away until you scroll near them.
- **Each picture is fetched once.** Covers, banners, icons and avatars are kept on disk (`%LOCALAPPDATA%\dev.idlekit.app\pictures`) and shown from there afterwards, without touching the internet. A picture Steam turns out not to have is remembered too, for three days, and not asked for again at every start.
- **Your profile picture shows up.** It is taken from Steam's own copy on this PC when there is one, so it needs no network at all. An account whose profile couldn't be read when it was added, which left it without a picture for good, is now tried again at each start.
- While a picture is on its way, the tile shows the game's name instead of staying blank.
- The log gets one line per run of fetching ("Fetched 24 pictures from Steam, …"), not a line per picture.

## 0.5

Releases are now numbered 0.5, 0.6 and so on, one step per release; after 0.9 comes 1.0. This one follows 0.2.2.

- **The log keeps more out.** Shortened IPv6 addresses, a Steam ID written right next to other text, and your Windows user name in any folder path (not only under `C:\Users`) are now replaced too.
- **The log garbles less.** The line for signing in by name and password is whole again. A folder path no longer swallows the rest of its sentence, version numbers are no longer taken for network addresses, long names made of words (achievement IDs, market items, Steam's web addresses) are no longer hidden as if they were keys, and a sign-in name that happens to be an everyday word such as "steam" is no longer blanked out everywhere.
- When `log.txt` starts over at 2 MB, the new file begins by saying again which version is running on which system. If the full file can't be set aside, it is emptied instead of growing without end.
- A line written at the very moment the log file is opened can no longer miss the file.
- "The window is ready" is now written once the first page is on screen, and a start that fails is logged. The check each setup file goes through before it is published waits for that line and fails if the log reports a fault in the window or the app.

## 0.2.2

- **A log file.** IdleKit now writes what it did and what went wrong to `log.txt`, so there is something to look at after a problem even when the app was closed or crashed. The Logs page has a **Show log.txt** button that opens its folder (`%APPDATA%\dev.idlekit.app`). The file starts over at 2 MB, keeping the previous one as `log.old.txt`.
- **Every action is written down:** each thing done in the window with how long it took, or the error it ended with; changed settings, value by value; what the close button and the tray menu did; each run's IdleKit version, system and web view version, and whether it was started at sign-in; and faults in the window or the app that nothing else caught.
- **Nothing private goes into it.** Sign-in names, Steam IDs (only the last four digits are kept, as "account …1234"), passwords, Steam Guard codes, cookies, tokens, QR sign-in links, e-mail and network addresses, custom profile addresses and your Windows user name are replaced before a line is stored, shown or written. The log does say which games (by number and name), achievements and market items an action was about.
- The check each setup file goes through before it is published now also makes sure the window's contents load, that the log file is written, and that it holds neither the user name nor the computer name.

## 0.2.1

- **Start on launch waits for Steam.** Started at sign-in, IdleKit is usually up before the Steam client is. Auto-idle and card farming used to try once, fail and never start; they now wait up to five minutes for the Steam client (or, for direct accounts, the connection).
- **Achievements without the wait.** Opening a game in the manager, and every change or unlock, waited up to 12 seconds for a signal that the current Steamworks SDK no longer sends. A game without achievements now shows an empty list instead of an error.
- **Card farming** no longer takes a game's foil badge for the game itself, which could end that game's farming early. When none of the games could be started it says so instead of "finished".
- **Playtime booster** counts its minutes from when the boost is set, not from when the game began idling.
- **Market in any wallet currency.** Fees, the lowest possible price and the step between prices now come from the numbers Steam publishes for your wallet, instead of assuming US dollars.
- **Windows:** after Steam crashes, IdleKit no longer believes it is still running.
- Smaller fixes: a direct account's games are no longer shown as idling when Steam couldn't be told about them; a pasted cookie is stored as just the cookie; removing an account also removes its Steam Guard token; the unlocker reports achievements with "/" in their name correctly; a failed update says that idling was stopped for it.

## 0.2.0

- **Runs in the tray.** Closing the window can keep IdleKit going in the system tray. The first time, it asks; the choice is under Settings, Running in the background.
- **Starts with Windows**, optionally straight into the tray, so idling and card farming can begin without opening anything.
- **Updates itself.** IdleKit looks for a newer release when it opens and installs it when you say so.
- **Game art.** Covers in the library (or wide banners) and each game's icon in every list.

## 0.1.0

- Idling and playtime booster, card farming, achievement manager and unlocker, market seller, and multiple accounts with three ways to sign in.
