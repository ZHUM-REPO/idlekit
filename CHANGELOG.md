# Changelog

The section for the version being released becomes the text of its release page.

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
