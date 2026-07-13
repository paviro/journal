# Running on iOS and Android

Notema runs on iOS/iPadOS through [iSH](https://ish.app) and on Android through
[Termux](https://termux.dev). There is no background daemon on either, so a file
sync tool moves the journal in and out and Notema reads it locally.

## iOS / iPadOS (iSH)

iSH emulates a 32-bit Linux, so grab the **`linux-musl-i586`** binary from the
[releases page](https://github.com/paviro/notema/releases). It has to be the i586
build, not i686 — iSH's emulated CPU has no SSE2, and the i586 baseline is compiled
for x87 floats so it runs there. The musl binary is statically linked and runs
as-is.

```sh
unzip linux-musl-i586.zip
chmod +x notema
mv notema /usr/local/bin/     # iSH home is /root/
```

### Syncing with SyncTrain

[SyncTrain](https://apps.apple.com/app/synctrain/id6553985316) is a Syncthing
client for iOS. Add your journal folder as a synced folder and store it in
SyncTrain's own container (the **Default folder** location) — selective sync on or
off both work. Keeping it in the container is what lets iSH reach it.

### Opening the journal

Just run `notema`. On first launch (and again whenever nothing is mounted yet) it
mounts the iOS folder for you at `/mnt/Journals` by running `mount -t ios .`, which
pops the iOS Files picker — pick the SyncTrain folder that holds your journal.

The first time, Notema scans the selected folder, reports the journals and entry
count it found, and asks you to confirm it as this device's journal folder. That
scan can be slow on iSH; it only runs once, and later starts are fast. Your answer
is remembered in `ish-store.toml`, so on every later launch Notema re-mounts,
checks you picked the *same* folder, and refuses to touch a different store by
mistake.

On each launch Notema checks whether the folder is still mounted and only re-runs
the mount (re-showing the picker) when it isn't. The mount often survives a
restart, but iOS revokes the folder's access at unpredictable points, so the
picker resurfaces from time to time — just reselect the same folder. You never
type or configure a path by hand.

Sometimes right after the TUI launches iSH won't register key presses until you
tap the terminal once. If nothing responds, tap the screen and try again.

### Current location

`Ctrl+L` grabs the device's location on iOS too, read from iSH's `/dev/location`
GPS stream. iSH prompts for Location access the first time — allow it. If you
denied it earlier, re-enable it under iOS Settings › Privacy › Location Services ›
iSH, otherwise no fix arrives.

### Keeping things in sync

iOS heavily limits background work, so SyncTrain does not sync continuously. After
you write or edit entries, **open SyncTrain** to push the changes out (and to pull
in changes made elsewhere before you start writing).

iSH does not support live file watching. Notema refreshes automatically on each
startup; press `r` in the TUI to refresh external changes immediately.

## Android (Termux)

Grab the **`android-aarch64-termux`** binary from the
[releases page](https://github.com/paviro/notema/releases).

```sh
unzip android-aarch64-termux.zip
chmod +x notema
mv notema $PREFIX/bin/
```

### Syncing with Syncthing

Use [Syncthing-Fork](https://github.com/Catfriend1/syncthing-android) (or run
`syncthing` inside Termux directly). With Syncthing-Fork, keep the journal in
normal shared Android storage so both the sync app and Termux can see it.

Give Termux access to shared storage:

```sh
termux-setup-storage
```

This mounts shared storage into your home directory at `~/storage/shared`. See the
[Termux wiki](https://wiki.termux.com/wiki/Termux-setup-storage) for details. Put
the journal in a shared folder (e.g. sync it there with Syncthing-Fork) and point
Notema at it under `~/storage/shared/`.

The Termux home directory itself is at `/data/data/com.termux/files/home`
(`$HOME`), and shared storage at `~/storage/shared` →
`/storage/emulated/0`. Use the journal path when Notema asks for it on first run,
for example:

```
Journal root [Journals]: ~/storage/shared/Journals
```

Unlike iOS, Syncthing on Android can run in the background, so edits generally sync
without manual intervention. How reliably depends on your battery/background
settings, though, so check the sync completed before relying on it.
