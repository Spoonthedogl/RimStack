<p align="center">
  <img src="site/img/icon-512.png" alt="" width="128">
</p>

<h1 align="center">RimStack</h1>

<p align="center">
  <b>The RimWorld mod manager that is up in 0.2 seconds,<br>sorts by reading what is inside your mods,<br>and plays their XML patches before the game does.</b><br>
  Free, open source, native.
</p>

<p align="center">
  <a href="https://github.com/TheForgotten69/RimStack/releases"><b>Download</b></a> ·
  <a href="https://theforgotten69.github.io/RimStack/">Website and guide</a> ·
  <a href="https://github.com/TheForgotten69/RimStack/issues">Report a problem</a>
</p>

<p align="center">
  <img src="site/img/mods.png" alt="RimStack's Mods screen: the load order in categories, with Sort ready to move six mods" width="720">
</p>

## Fast, for real

Same machine, same install (1,853 mods, 101 active), launch to mod list on screen:

| | RimStack | RimSort 1.15.0 |
|---|---|---|
| Mod list on screen | **0.2 s** | 7.0 s |
| Window up | **0.27 s** | 0.65 s |

About 35 times faster to your mod list. RimSort's numbers are the average of three runs, from launch until it logs that its window started and that its lists are filled. RimStack's come from its own timer from process start (it prints `mods_shown_ms`). Every mod's `About.xml` is read in 0.12 s.

RimStack is a native app, and it never re-runs a check it already did, so filtering, scrolling, sorting and saving stay instant with thousands of mods. Run both on your own install and see.

## Plays your patches before the game does

Load order matters most where two mods patch the same thing. Take two mods that both give a xenotype the same gene, one of them only if the gene isn't there yet. Load them one way and the gene is there once. Load them the other way and it's there twice. The game says nothing, and you find out in play.

RimStack finds it before you launch. In the background, it builds what the game would load: every active mod's defs in one document, every XML patch applied in load order with the game's own rules, then inheritance. Then it looks for three things that go wrong:

- **A patch operation that fails**, as the game would log it.
- **An element doubled in a def**, which the game logs as a duplicate node.
- **A def named twice in a list**, such as a gene listed twice in a xenotype. The game never reports this one.

When moving two mods removes an error without bringing a new one, Sort makes that move and names the def and the patch behind it. An error that no order avoids is shown as a warning on the mod, naming the patch file and how many of its operations fail. Patch classes from other frameworks are followed only as far as their XML says, and an error on a def they might change is never reported, so a warning is never a guess.

It takes about a second on a hundred mods, and the result is kept until a mod's files, the list or your rules change. If you press Sort while it's still running, Sort waits for it.

## Live, always

You never press Refresh. While the window is open, RimStack looks at your mods, saves and the game every two seconds and updates by itself:

- **Steam downloads and updates.** Subscribe, unsubscribe or let Steam update a mod, and it appears, goes, or shows its new version.
- **Mods changed by hand.** A folder copied into Mods, an `About.xml` edited, a Git checkout pulled.
- **The game's own list.** If the game or another tool rewrites `ModsConfig.xml`, you're offered that list, never forced onto it.
- **Saves and logs.** A new save shows up in Saves; when the game closes, its log is read and its errors grouped.
- **Every setting.** Theme, language, text size, font, your own colours and translations apply the moment you change them.

## Sorts by what is inside

Most managers sort with the rules mod authors wrote down. Plenty of mods never wrote any, so you end up sorting by hand and guessing.

RimStack opens the mods' compiled DLLs and reads them. It never runs anything. It sees which game methods each mod patches with Harmony (prefix, postfix, transpiler, finalizer) and uses that to find load order rules nobody declared. It reads XML patches the same way. Every move Sort makes is shown with the reason first, and nothing is applied until you say so.

What it learns from playing the XML patches is [described above](#plays-your-patches-before-the-game-does).

It also sorts your mods into categories on its own (loaders, game and DLC, libraries, content, patches, textures, interface, performance), again from what is in the mod and never from its name. You can override any of them.

## Nothing reaches the game before Save

Moving, activating, sorting and loading a list only change the list inside RimStack, and every step can be undone. Save writes one file, the game's `ModsConfig.xml`, after copying the old one beside it.

Everything else that writes happens only when you ask: converting textures, hiding a texture that loses to another mod's, downloading, unsubscribing, or setting mod settings aside. Every file RimStack makes or hides is recorded so it can be undone. Play also copies your newest save into RimStack's own folder first, just in case.

## What else is in it

- **Explains every problem.** Missing requirements, mods that clash, the wrong game version, duplicates, a mod placed above the game, each with its fix beside it.
- **Texture conflicts.** Two mods ship the same texture? See them side by side, as a slider or as their difference, and keep the one you want.
- **PNG to DDS built in.** The game loads faster, with no extra tool, and only the files RimStack made are ever removed.
- **Find the broken mod.** It switches half your mods off, you play and say whether it still breaks, until one mod is left. Your list comes back after.
- **Reads the game's errors.** Thousands of log lines become a few errors, each with the mod its stack trace points at and a report to copy.
- **Saves.** Which mods each save was played with, what is missing, and its list as your load order in one click.
- **Downloads and updates.** Paste Workshop, collection or Git links to subscribe, fetch with SteamCMD, or clone. See which mods are behind, and what changed since the game last started.
- **Filters, saved lists, tags and notes, Undo and Redo,** and every action from one search box (Ctrl + K).
- **Coming from RimSort?** The first run imports your rules, tags and notes.
- **Seven languages:** English, Deutsch, Español, Français, Italiano, Polski and Português.

Every panel is explained, with pictures, in the [guide](https://theforgotten69.github.io/RimStack/#guide).

## Install

Grab the file for your system from the [Releases page](https://github.com/TheForgotten69/RimStack/releases).

| System | File |
|---|---|
| Windows | Installer (`-setup.exe`) or zip |
| macOS | dmg, Apple silicon only |
| Linux | AppImage |
| Arch (AUR) | Coming soon |

RimStack finds RimWorld through Steam. If it can't, it asks you for the folder.

### The apps are not signed yet

Signing costs money, so for now Windows and macOS will warn you. The files are fine, here is how to get past it once.

**Windows.** SmartScreen shows "Windows protected your PC". Click **More info**, then **Run anyway**. For the zip, right-click it, choose **Properties**, tick **Unblock**, then extract. If Smart App Control is on (Windows 11), it blocks unsigned programs with no way past it except turning Smart App Control off.

**macOS.** It says Apple could not verify the app is free of malware.
1. Drag RimStack to Applications and open it once.
2. Open **System Settings, Privacy & Security**, scroll down, and click **Open Anyway** next to RimStack.
3. If macOS says the app is damaged, run this in Terminal: `xattr -dr com.apple.quarantine /Applications/RimStack.app`

**Linux.** Make the AppImage executable and run it: `chmod +x RimStack-*.AppImage`.

## Licence

GPL-3.0-or-later. See [`LICENSE`](LICENSE).

Additional permission under GNU GPL version 3 section 7: if you modify this
program, or any covered work, by linking or combining it with the Steamworks
SDK (or a modified version of that library), containing parts covered by the
terms of the Steamworks SDK licence, the licensors of this program grant you
additional permission to convey the resulting work.
