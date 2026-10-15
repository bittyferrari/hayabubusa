# Hayabubusa — A Lightweight, Fast Image Viewer

**English** | [繁體中文](README.zh-TW.md)

A single-executable image viewer written in Rust + egui, for Windows / macOS (Linux works too).
Goal: images open instantly, press Enter to toggle between "single image" and "folder thumbnails" just like XnView, and keep only the tools you actually use.

## Features

- **Single image view**: decoding starts in the background at launch (in parallel with window creation), and the previous / next images are preloaded, so switching is nearly instant
- **Comic mode**: all images of the folder stacked top to bottom in one continuous strip, all at the same width — scroll through them like a webtoon / manga reader, zoom in and out (the zoom level is remembered). Image sizes are read from the file headers in the background, so the layout is right before anything is decoded. Images are decoded already scaled to the width they are shown at (JPEGs in the DCT domain), starting with what is on screen and then about two screens ahead in the scroll direction, and at most one new texture is uploaded per frame — so scrolling stays smooth
- **Folder thumbnails**: multithreaded thumbnail generation, processing only what's visible on screen (plus two rows of look-ahead)
- **Auto refresh**: the folder is checked in the background every second; any added / deleted / modified files (including changes made by other programs) are picked up automatically while keeping your selection. Modified files get new thumbnails, and thumbnails that failed because a freshly copied file was temporarily locked are retried automatically
- **Sorting**: choose Name (natural sort) / Modified time / Created time / File size / Type from the toolbar, ascending or descending. Folders always come first, previous / next in single image view follows the same order, and the setting is remembered
- **Thumbnail cache**: stored as WebP (quality 80, 256px long edge), about 2–8 KB each
  - Windows: `%LOCALAPPDATA%\hayabubusa\thumbs`
  - macOS: `~/Library/Caches/hayabubusa/thumbs`
  - Keyed by "path + file size + modified time", so thumbnails are regenerated automatically when a file changes
  - JPEG thumbnails use DCT-scaled decoding (decoding directly at 1/8 size), which is much faster
- **Cut / Copy / Paste / Delete**: shares the clipboard with File Explorer, so you can paste in either direction. Paste and delete go through the Windows shell (progress dialog, automatic renaming on name conflicts, deleted items go to the Recycle Bin and can be restored). Windows only for now
- **Rotate** (left / right)
- **Convert**: PNG / WEBP / JPG (adjustable quality; WEBP quality 100 = lossless)
- **Resize**: by percentage, long edge, fit within width/height (keep aspect ratio), or exact width/height
- Supported formats: jpg / png / webp / gif (first frame) / bmp / tiff / ico / tga, with EXIF orientation applied
- CJK file names: system fonts are loaded automatically (Microsoft JhengHei / Yu Gothic / Malgun Gothic / PingFang, etc.), with a Korean font added automatically when needed
- Drag files or folders onto the window to open them
- **Multilingual UI**: Traditional Chinese / English / 日本語 / 한국어. Follows the system language by default; change it under "⚙ Settings → Language" (takes effect immediately and is remembered)
- **Stay resident in the system tray (Windows)**: closing the window doesn't actually exit — it hides to the system tray. Opening another image hands it straight to the running Hayabubusa (the new exe exits as soon as it finds it, without creating a window or initializing the GPU), so it opens almost instantly
  - By default it exits automatically after 5 minutes without opening an image (adjustable in "⚙ Settings"; 0 = never exit automatically; resident mode can also be turned off entirely)
  - Tray icon: left-click to show the window; the right-click menu offers "Open Hayabubusa" or "Exit"
  - Memory used by images and thumbnails is released while hidden

## Keyboard Shortcuts

### Folder Thumbnails

| Key | Action |
|---|---|
| Arrow keys / PageUp / PageDown / Home / End | Move |
| Shift + Arrow keys, Shift + Click | Range select |
| Ctrl + Click | Multi-select |
| Ctrl + A | Select all |
| **Ctrl + C** / **Ctrl + X** | Copy / cut the selected files or folders; paste them in File Explorer or on the desktop (cut items are dimmed) |
| **Ctrl + V** | Paste files from the clipboard (copied / cut in Hayabubusa or File Explorer) into the current folder |
| **Delete** | Move the selected items to the Recycle Bin |
| **Enter** / Double-click | Open image or enter folder |
| Backspace | Go up one level |
| F5 | Refresh (reloads the list and all thumbnails; also works in single image view) |
| Right-click | Open / Cut / Copy / Paste / Delete / Rotate / Convert menu |
| Mouse wheel | Scroll (rows per notch adjustable in "⚙ Settings") |
| Ctrl + Mouse wheel | Change thumbnail size |

The toolbar's "Rotate left / Rotate right" **overwrites** the selected files directly.

### Single Image View

| Key | Action |
|---|---|
| **Enter** / Esc | Back to folder thumbnails (cursor stays on the current image) |
| → ↓ PageDown Space / Wheel down | Next image |
| ← ↑ PageUp Backspace / Wheel up | Previous image |
| Home / End | First / last image |
| Ctrl + Mouse wheel, + / - | Zoom (centered on the mouse position) |
| Drag | Pan |
| 0 or Z | Fit to window |
| 1 | 100% |
| E / Q (or L) | Rotate right / left (preview first) |
| **Ctrl + S** | Save rotation |
| Ctrl + C | Copy the current image (as a file); paste it on the desktop |
| Delete | Move the current image to the Recycle Bin and go to the next one |
| R | Convert / Resize dialog |
| F, F11, Double-click | Fullscreen |

Rotation in single image view is only a preview; press Ctrl+S to write it to the file. Moving to another image without saving discards it.

### Comic Mode

Open it with **C** (or the "📖 Comic mode" button) in single image view, or the "📖 Comic mode" button in the folder toolbar (starts at the selected image). Once on, opening an image keeps using comic mode until you switch back with C.

| Key | Action |
|---|---|
| Mouse wheel / ↑ ↓ | Scroll (amount per notch adjustable in "⚙ Settings") |
| PageDown / Space, PageUp / Shift+Space | Scroll one screen |
| → / ← (Backspace) | Next / previous image (aligned to the top) |
| Home / End | Beginning / end |
| Ctrl + Mouse wheel, + / -, toolbar − + | Zoom (centered on the mouse position; remembered) |
| 0 or Z | Fit width |
| 1 | 100% (of the current image) |
| Drag | Scroll / pan |
| **C** | Back to single image view (on the current image) |
| **Enter** / Esc | Back to folder thumbnails |
| Ctrl + C, Delete, R, F / F11 / Double-click | Same as single image view, on the current image |

The "current" image is the one a third of the way down the screen; it is what Enter / C / Delete / Ctrl+C act on.

## Settings

"⚙ Settings" in the folder thumbnail toolbar:

- UI language (Follow system / 繁體中文 / English / 日本語 / 한국어)
- Thumbnail rows scrolled per wheel notch (default 1 row), smooth scrolling on/off
- Comic mode: pixels scrolled per wheel notch / arrow key (default 150 px; also reachable from the comic mode toolbar)
- Confirm before deleting (on by default; checking "Don't ask again" in the confirmation dialog also turns it off)
- Window on open: remember last state (size / position / maximized; maximized the first time) or always maximized
- Stay in the system tray after closing the window (on by default, Windows), and exit automatically after N minutes without opening an image (default 5 minutes)

Settings file: Windows `%APPDATA%\hayabubusa\settings.txt`, macOS `~/Library/Application Support/hayabubusa/settings.txt`

## Building

### Windows

1. Install [Rust](https://rustup.rs/) (the installer will prompt you to install the Visual Studio C++ Build Tools — just follow along)
2. Run in this folder:

   ```
   cargo build --release
   ```

3. Output: `target\release\hayabubusa.exe` (a single file, about 10 MB, can be copied anywhere)

**Set as default image viewer**: right-click any jpg → "Open with" → "Choose another app" → select `hayabubusa.exe` → check "Always use this app". Repeat for png / webp.

### macOS

```
cargo build --release
./target/release/hayabubusa image.jpg
```

Note: opening from Finder's "Open With" on macOS requires packaging as a `.app` and handling Apple's open-file events, which isn't implemented yet. For now, drag files onto the window or open them from the command line.

### Automatic Builds with GitHub

Move `github-build.yml` to `.github/workflows/build.yml` and push the folder to GitHub. Windows and macOS (Intel / Apple Silicon) builds will be produced automatically and can be downloaded from the Actions page.

## Notes

- Rotating and overwriting re-encode the image: JPEG is saved at quality 95, and EXIF and other metadata are not preserved
- Files are written to a temporary file first and then renamed into place, so a failed write won't corrupt the original
- To clear the thumbnail cache, just delete the thumbs folder mentioned above

## Project Structure

```
src/main.rs      Startup: parse arguments, start decoding early, open the window
src/app.rs       UI: folder thumbnails, single image view, dialogs
src/app/comic.rs Comic mode (vertical strip of all images)
src/workers.rs   Background thread pool (newest requests first, scrolled-away jobs dropped) and thumbnail cache
src/decode.rs    Decoding, EXIF orientation, fast JPEG thumbnails, WebP encode/decode
src/ops.rs       Rotate / convert / resize / batch processing
src/fonts.rs     Load system CJK fonts per language
src/i18n.rs      Localization: language detection and per-language strings
src/clipboard.rs File lists on the system clipboard (Windows CF_HDROP)
src/fileops.rs   Copy / move / delete to Recycle Bin (Windows SHFileOperation)
src/resident.rs  System tray residency, single instance (path hand-off via WM_COPYDATA), idle auto-exit
src/util.rs      Natural sort (img2 < img10), hashing, settings file
```
