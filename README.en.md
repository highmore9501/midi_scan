# MIDI Manager

[English](README.en.md) | [中文](README.zh.md)

A local MIDI library organizer: scan the MIDI files on your computer, build a searchable library, find the songs you want quickly, and clean up duplicate files.

---

## Features

### Scanning & Organization

- Scan one or more folders for MIDI files (`.mid` / `.midi`).
- Automatically extract each file's: name, path, instruments used, and note count per instrument, then store them in a local library.
- **Incremental scanning**: re-scanning the same folder only processes new or modified files — no redundant re-parsing.
- **Live progress**: see the file currently being processed and running counters in real time. You can hit "Stop" anytime on very large folders; what's already processed is kept.

### Duplicate Handling

- Automatically detect and group duplicate files:
  - **Byte-identical** (copies);
  - **Structurally identical** (same instruments and note counts — possibly different versions of the same song).
- Review groups one by one, or use **"Resolve All"** to handle everything at once: each group keeps the earliest-scanned file by default, and the rest are deleted per your selection.
- A confirmation dialog always appears before deletion — **deletion is permanent and cannot be undone**, so please confirm carefully.

### Combined Search

- Filter the library by a combination of instruments and note counts.
- Two matching modes:
  - **Exact**: the file's instrument set is exactly the instruments you selected;
  - **Contain**: the file contains all the instruments you selected, and may contain additional ones.
- Supports per-instrument note ranges, total note ranges, file-name keywords, and folder-prefix filters. Results are paginated and can be opened with your system's default program.

### Statistics

- File status statistics (scanned / duplicate candidates / deleted / missing / parse failures, etc.).
- Top 10 instrument distribution (regular instruments only; percussion subdivisions excluded).

---

## Using the Desktop App

Launch the app (`midi-manager.exe`). There are four pages at the top:

### 1. Scan

1. Click "Browse…" to pick a folder, or type a path and press Enter. Multiple folders can be added.
2. Click "Start Scan" and watch the live progress. You can click "Stop Scan" anytime on large folders.
3. When done, the summary (new / updated / skipped / failed) is shown, and duplicates are detected automatically.

### 2. Library

1. Check the instruments to filter by (a keyword box helps you find instruments quickly).
2. Choose the matching mode: Exact / Contain.
3. Optional: per-instrument note range, total note range, file-name keyword, folder prefix.
4. Click "Search" to view results, browse pages, and click "Open" to open a file with the default program.

### 3. Deduplication

- View all pending duplicate candidate groups.
- Click a group to expand and see its files; check the files to delete (the earliest-scanned file is always kept).
- When there are many groups, use **"Resolve All"**: it processes every group in one go with the default rule (keep the earliest-scanned file, delete the rest).
- A confirmation dialog appears before deletion; confirm to delete permanently.

### 4. Settings / Statistics

- Library path, file status statistics, and Top 10 instrument distribution.

---

## Command Line Usage

A command-line tool (`midi-mgr.exe`) is included and shares the same library as the desktop app:

```text
# Scan folders (multiple allowed)
midi-mgr scan --dir D:\Music\midi --dir E:\midi

# Exact search (instrument set exactly equals the selection)
midi-mgr query --instruments "Acoustic Grand Piano,Bass Drum 1" --note-min 0 --note-max 500

# Contain search (contains all selected instruments, more allowed)
midi-mgr query --instruments "Acoustic Grand Piano" --superset

# Additional filters: total note range, name keyword, folder prefix, JSON output
midi-mgr query --instruments Piano --total-min 100 --name "theme" --json

# List pending duplicate candidate groups (no deletion)
midi-mgr dedup --dry-run

# Interactively confirm deletion group by group
midi-mgr dedup

# Auto-resolve all groups: keep oldest / newest / shortest path
midi-mgr dedup --keep oldest

# Library statistics
midi-mgr db --info
midi-mgr db --stats
```

---

## Notes

- Library data is stored at: `%USERPROFILE%\.midi-manager\library.sqlite` (shared by the desktop app and the CLI).
- Deleting duplicate files is **permanent** — deleted files are not moved to the Recycle Bin. Confirm before you act.
- For very large folders (hundreds of thousands of files), scan in batches; you can stop anytime and the processed part is kept.
