# Changelog

The section for the version being released becomes the text of its release page.

## 2.0

- **Select all from the head of the inventory table.** On the Market's Inventory tab, the box at the top of the checkbox column ticks every item the table shows, under the filter and search in use, and unticks them again once they're all ticked. While only some are ticked it shows a dash. Items picked under another filter or search stay picked.

## 1.9

- **Upcoming Events.** A new page in the sidebar lists Steam's upcoming sales and fests: the seasonal sales, Next Fest and the themed fests, grouped by month, with their days and times in your time zone and how long until each starts. The list comes from Valve's own schedule on the Steamworks site, is read again every day at midnight (or when you press Refresh), and is kept for when the site can't be reached.
- **The next event, in the sidebar.** A square above Accounts shows the sale or fest that's on, or the next one: the days to go (or left), its name and its days. Clicking it opens Upcoming Events. In a short window it shrinks to a few lines.
- **Card Farming, with a capital F**, in the sidebar, the top bar and on its page, like the other names there.

## 1.8

- **Seven more themes.** Four dark ones: Obsidian (true black, for OLED screens), Harbor (deep blue-slate), Moss (green-grey) and Plum (purple). And three light ones: Paper (warm cream), Frost (cool blue-white) and High contrast (black on white with strong lines). Settings → Appearance shows the dark themes in one row and the light ones in the next.
- **Ten running colors, and any color you pick.** Orange, Orchid, Blue, Mint and Silver join Amber, Rose, Violet, Cyan and Lime. On a light theme each one takes a deeper shade that reads on a light background, and the swatches show the shade that will be used. The last swatch opens a color picker for any color. The window takes the color while you pick, and text on it turns black or white, whichever reads better.
- **The Market loads by itself.** Once the account has a web session, opening the Market loads your inventory and your listings, with no button to press. Signing in to a web session while the Market is open loads them too. They stay loaded while you look at other pages or switch accounts and back; Refresh loads them again. When Steam doesn't answer, you see the reason once, not once for each, and the Market tries again the next time you open it.

## 1.7

- **The tray icon warns when the Steam client isn't signed in.** While the account selected in IdleKit uses the Steam client and the client isn't signed in to it, the tray icon wears a small amber warning sign in its bottom-right corner, and its hover text says "Steam client not signed in". The sign goes away within a few seconds of Steam signing in.

## 1.6

Fixes from a review of the whole app, for card farming above all.

**Card farming**

- **A game Steam won't start is tried again.** It used to be skipped for the rest of the run. Now the next game is farmed meanwhile, and the one that failed gets two more tries, two minutes apart.
- **One game's card page can't stop the run any more.** When a game's card page kept failing, the whole run stopped after five checks with "the web session may have expired". Now IdleKit looks at your badge page to tell the two apart: when Steam answers that one, only the game whose page fails is set aside, after three checks, and the others are farmed. When Steam doesn't answer at all, farming keeps idling and checks again later, as cards keep dropping while Steam's pages are down. A run only stops when Steam says the sign-in has ended, three checks in a row, and then it says to sign in again.
- **Steam's "An error was encountered" page no longer counts as "no drops left".** That page comes with an OK status and no drop count. It used to mark the game as done, with a card counted that never dropped. Now only a page that says how many drops are left counts, and anything else is checked again. The badge pages are read the same way, so a broken page can no longer end a run at once with "Finished — 0 cards" or leave games out.
- **"Finished" names any game that couldn't be farmed**, and the list shows those games as "Couldn't farm", with the reason.
- After a game fails to start, the next one starts straight away instead of 30 seconds later.

**Connections and startup**

- **A direct connection can't hang on "Connecting…" any more.** Finding Steam's servers, connecting and signing in now get 30 seconds; one that hangs counts as failed and is tried again like any other. Password sign-ins get the same limit for reaching Steam.
- **Steam's list of servers is kept** for an hour, and on disk between starts, so reconnecting is quicker and still works while Steam's directory is down.
- **Each account starts on its own at launch.** One that isn't ready, such as a slow connection or a Steam client signed in to another account, no longer holds up auto-idle or card farming for the others. An account the Steam client isn't signed in to stops being waited for after 30 seconds, where it held everything up for five minutes.
- **Web sessions refresh one account at a time each**, so a slow refresh for one account doesn't hold up farming or the market for the others.
- **Auto-idle starts without waiting for the games list.** The names follow once the list is in.
- **Page loads try again when a connection can't be opened or drops on the way**, after 3 and then 8 seconds, and a connection that can't be opened gives up after 10 seconds instead of 30.
- The look for updates at startup runs while accounts connect and wait for Steam, and isn't repeated eight seconds later.

**Achievements**

- **Closing the window to the tray closes an open achievement list.** An open list keeps its game running in Steam, so with the window in the tray you showed as in-game, with playtime counting, and automatic updates waited for the list to close. The list opens again when the window comes back.
- **A game's achievements open sooner:** the game starts in Steam while its global unlock rates download, instead of one after the other, and the rates are kept for an hour.

**Also**

- Automatic updates wait while a sign-in is under way (a QR code on screen, or a Steam Guard code being typed).
- The settings file is written to disk in full before it replaces the old one, and the copy before is kept as `config.json.bak`. If `config.json` is ever empty or missing, for instance after a power cut during a save, IdleKit uses that copy instead of starting with no accounts.
- Pictures are read from disk away from the window's own thread, so a big library scrolls smoothly on a slow disk.
- The market saves what it looks up once per run instead of after every item, and doesn't wait after the last item.

## 1.5

- **Updates install by themselves.** When a newer version is found and nothing is running, IdleKit installs it and restarts, without asking. “Nothing running” means no idling, no card farming, no unlocking, no market work and no open achievement list. While something is running it waits, checks once a minute, and the bar offers “Install and restart now”. There is a switch for it under Settings → Updates; it is on.
- **At startup the update comes first.** IdleKit looks for a newer version before it starts anything, so a PC that starts IdleKit at sign-in gets the new version before auto-idle or card farming begin. If it was started in the tray it comes back in the tray; if its window was open it comes back with the window open.
- **It looks every half hour**, where it was every four hours.
- **A look that fails is repeated much sooner:** once more after a few seconds, then after one, two and five minutes, then every ten. Before, the gaps were ten minutes three times and then an hour, which is why a new version could take a long time to show up on a connection that drops requests.
- **The log says why a look failed.** “error sending request” is now followed by the reason, such as a timeout or a name that couldn't be found.
- If an automatic install doesn't take, IdleKit doesn't try that version by itself again for twelve hours, so it can't restart over and over.

## 1.4

- **Pages come back as you left them.** Opening another page used to reset a page to its defaults: the Library went back to “All games” sorted by “Name”, the Market to its Inventory tab, the Logs to everything.
- **Saved across restarts:** the Library's filter and sort, the Market's tab and item filter, and the Logs page's level. They are kept with the settings.
- **Kept until IdleKit closes:** search text on every page, the Library's “Only NSFW games”, and on the Achievements page the Manager or Unlocker tab, the achievements or stats view and the locked or unlocked filter. A fresh start clears these, so a search or the NSFW games don't come up by surprise.

## 1.3

- **An NSFW box in the Library**, beside the sort box, with three choices:
  - **NSFW games hidden** and **NSFW games shown** switch “Hide NSFW games” on and off, the same setting as the switch in Settings, so it applies to every page.
  - **Only NSFW games** shows just the games Steam labels “Adult Only Sexual Content” or “Frequent Nudity or Sexual Content”, to see which ones they are. It is a look at the Library only: the setting stays as it was, and the box goes back to it when you leave the page.

## 1.2

- **Games shown as “App 1030300” now get their names.** An account that uses the Steam client on this PC, with no web sign-in, gets its games list from Steam's files. Those name the games that are installed, but list a game that was played and is no longer installed by its number only, and Refresh read the same files again. IdleKit now asks Steam's store for the names of those games, with the lookup that already finds their covers, so it needs no sign-in. The names appear a moment after the list the first time and are kept after that.
- A name the game already has, from Steam's files or one you typed when adding a game by AppID, is never replaced. A game Steam's store has nothing on keeps its number.
- A game added by AppID without a name gets its name the same way.

## 1.1

A copy of IdleKit that was left running never noticed new releases. This release fixes that. **A copy older than 1.1 still has the fault, so install this one by hand once** (or press the check button under Settings → Updates in the old copy).

- **IdleKit keeps looking for updates while it runs.** It used to look once, a few seconds after starting. A copy left open or in the tray for days never looked again, and a look that failed, for instance on a busy connection at startup, was not repeated. Now it looks every four hours, and after a failed look again in ten minutes.
- **A newer version stays announced.** Besides the bar under the top bar, the version line at the bottom of the sidebar says “1.1, 1.2 available” for as long as that is so, also after the bar was closed. Clicking it opens Settings, where the update is installed. Updates are still never installed without asking.
- **The version is shown beside the name** at the top left of the window.
- Every look for updates now leaves a line in the log, also when nothing newer was found, so `log.txt` shows whether and when it looked.

## 1.0

- **Hide NSFW games.** A new switch under Settings → Appearance. When it is on, games that Steam labels “Adult Only Sexual Content” or “Frequent Nudity or Sexual Content” are left out of every list in the window: the Library, Idling, Card farming, the Achievements manager and unlock queue, and the Market's inventory and listings. Games Steam labels only “Some Nudity or Sexual Content”, such as many mainstream games, stay.
- Hiding is about what the window shows. A hidden game that is on the auto-idle list, in the unlock queue or has card drops left keeps running; the pages say how many games are not shown.
- What a game contains comes from Steam's store, with the lookup that already finds its pictures, and is remembered on disk. With the switch on, a game is shown only once Steam has answered for it, so the first time the lists fill in after a moment. If Steam can't be reached, the page says how many games are waiting and offers to ask again.
- Lines on the Logs page that name a hidden game are left out there. `log.txt` still has them.
- It is off unless you turn it on.

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
