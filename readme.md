# mus_renamer

Arranges a music folder by what the tags say — for a car stereo, or just for a
collection that grew out of hand.

```
incoming/kino/*.mp3   →   Kino - 1988 (Gruppa krovi)/
rip1/*.mp3            →   Metallica - 1986 (Master of Puppets)/
rip2/*.mp3            →   Metallica - 1986 (Master of Puppets) (2)/
hits/*.mp3            →   VA - Disco Hits 99/
mixed/*.mp3           →   left alone: 2 artists and 2 albums in one folder
```

Rust rewrite of the old Python tool. The Python version lives on in git history
(`git show 9efe894:mus_renamer/main.py`).

## How it decides

Per folder, from the tags of the tracks inside it:

| distinct artists | distinct albums | verdict                              |
|------------------|-----------------|--------------------------------------|
| 1                | 1               | album → `Artist - Year (Album)`      |
| 1                | many            | collection → `Artist - collection`   |
| many             | 1               | compilation → `VA - Album`           |
| anything else    |                 | **left alone**                       |

A folder that holds sub-folders is a container, not a release, and is skipped.
A track with no artist/album tag makes its folder ambiguous by default — the
tool never guesses. Every one of these rules is a setting (see `mp3rnm config`).

Names are transliterated from Cyrillic per character (no language detection),
stripped of everything FAT and NTFS forbid, and capped at 255 bytes.

## Safety

The whole point of the rewrite. **No operation can overwrite or destroy data:**

* **Dry run by default.** Nothing happens without `--execute`, and `--execute`
  asks for confirmation unless you pass `-y`.
* **Nothing is written over.** Moves use `renameat2(RENAME_NOREPLACE)`, so an
  existing destination is a kernel-level impossibility, not a race we hope to
  win. Where the flag is unsupported, the destination is checked first and the
  result verified after. A name that is taken gets ` (2)`, ` (3)`, …
* **Nothing is deleted.** `clean` *moves* junk into `<root>/.mus_renamer/trash/`.
  Only empty directories are actually removed, with `rmdir`, which fails on
  anything that still holds data.
* **Every move is proved.** The source's file tree — names and sizes, plus a
  blake3 of every byte with `--verify hash` — is captured before the move and
  compared after it. A mismatch fails the move and rolls it back.
* **Copies are verified before the original goes.** Across filesystems
  (`--cross-device copy`) the data is copied to a staging directory, hashed
  against the original, put in place, and only then is the original removed.
* **A stale plan is refused.** Each planned move carries the identity the source
  folder had when the plan was built; a folder that changed in between is
  skipped, not moved on an outdated judgement.
* **Everything is journalled.** Each operation is written and `fsync`ed to
  `<root>/.mus_renamer/journal-*.jsonl` before it is attempted, so any run can
  be undone — including the undo.

## Install

```sh
cargo install --path crates/mus_cli      # the mp3rnm command
cargo install --path crates/mus_ui       # optional window: mp3rnm-ui
```

## Use

```sh
mp3rnm scan  /media/usb            # what does each folder look like
mp3rnm plan  /media/usb            # what would be renamed
mp3rnm apply /media/usb            # dry run: every check, no changes
mp3rnm apply /media/usb --execute  # do it (asks first)

mp3rnm undo  /media/usb --execute  # take the last run back
mp3rnm journals /media/usb         # what runs happened

mp3rnm clean /media/usb --non-audio --empty-dirs --execute   # junk to the trash
```

Plan now, apply later — useful when the plan is worth reading first:

```sh
mp3rnm plan /media/usb --out plan.json
mp3rnm apply --plan plan.json --execute
```

Common options:

```
--layout flat|artist-folders     Artist - 1999 (Album)/  or  Artist/1999 (Album)/
--album-template "{artist} - {year} ({album})"
--translit off|cyrillic|ascii
--ext mp3,flac                   what counts as audio
--legacy-encoding cp1251         re-decode tags written by old taggers
--artist-source album-artist     which frame names the artist
--untagged-artist ambiguous|unknown|ignore
--verify metadata|hash           how thoroughly a move is checked
--cross-device refuse|copy       when the destination is another filesystem
--config settings.toml           everything above, in a file
```

`mp3rnm config > settings.toml` prints every setting with its current value.

Exit codes: `0` fine, `1` something failed, `2` bad usage, `3` you said no.

### Window

`mp3rnm-ui [folder]` — the same thing with a plan you can look at and untick row
by row before pressing Apply.

## Layout

| crate      | what it is                                                              |
|------------|-------------------------------------------------------------------------|
| `mus_core` | tags, scanning, classification, naming, planning. Decides; never acts.   |
| `mus_fs`   | the only code that touches data: verified moves, trash, journal, undo.   |
| `mus_cli`  | `mp3rnm`                                                                 |
| `mus_ui`   | `mp3rnm-ui` (egui)                                                       |

`cargo test --workspace` — the interesting tests are in
`crates/mus_fs/tests/safety.rs`, which check *data conservation*: the multiset of
file hashes under the root is identical before and after every run, however the
folders were shuffled.

## Related tools

```sh
sudo mount /dev/sdb1 /mnt/1 -o rw,umask=002,codepage=866,iocharset=utf8,gid=100
duff -re0 . | xargs -0 rm     # deduplicate
fatsort /dev/sdX1             # FAT directory order is not alphabetical by itself
```
