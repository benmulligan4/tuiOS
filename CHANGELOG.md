# Changelog

All notable changes to this project are documented here, newest first. The project was originally named TUIX and was renamed to tuiOS on 2026-09-23. Entries are grouped by date; PR numbers refer to `benmulligan4/TUIX`. The crate version in `Cargo.toml` is still `0.1.0`, and no release tags exist.

## 2026-11-27

### Added
- Task manager bar, a compact system bar with tiles, its own design and colours, and a new Task Bar settings page for choosing item content (#25).
- Expanded the registered apps list in `config/registered-apps.json` (#25).

## 2026-10-27

### Added
- Favourite apps: mark apps as favourites from the app store (#24).
- Backup and restore: a settings backup module and a Backup settings page (#23).
- Task manager (#22):
  - Pages for performance, GPU (NVML support compiled on Windows only), graphs, network and network info.
  - A process/session control layer, keybinds and a switcher.
  - A task manager settings page, including an optional background process scan.
- `x/test-registered-apps.sh` and `.bat` scripts, which test registered apps in a temporary sandbox (#21).

### Changed
- The on-screen keyboard is disabled by default (#23).
- Removed the running-apps count from the System nav bar item (#22).
- The startup tab now opens in a new window (#22).
- The apps category submenu is enabled for the navigation bar menu item (#22).
- Third-party dashboard runner and dashboard 2 updated for the task manager (#22).

## 2026-10-26

### Added
- Navbar settings subpage and category ordering page. The Apps menu dropdown can be organised by category (#20).
- Notes app with a markdown renderer, editor and store (#18).
- Expandable panes in the app store (#19).
- Rainbow boot animation with static and nebula colour options, gradients, loading direction, a boot settings page and a live preview (#15).

### Changed
- Navbar dropdown closing behaviour updated (#20).
- App store (#17):
  - The Awesome Ratatui browser now parses all categories.
  - Added collapse/expand for categories and shift-to-jump to a category.
  - Added organise by category and sort by recently installed.
  - Added tuiOS app and tuiOS dashboard categories.
  - Improved pane, filter panel, queue and retry navigation.
  - Added open-browser with the right arrow.
  - Reordered the app details menu.
- Uninstall now works when apps are sorted by recently installed (#19).

### Fixed
- Search bar jumping in the Awesome Ratatui browser (#17).
- Dropdown settings not showing on Raspberry Pi (#15).
- Loading bar colour for the static rainbow animation (#15).

## 2026-09-25

### Added
- Modern boot animation using the tuiOS logo, alongside the retro one (#14).

## 2026-09-23

### Added
- Boot animation (#13).
- App store install queue (#11).

### Changed
- Renamed TUIX to tuiOS, including the `src/tuix` module, which is now `src/tuios`, and the `x/` scripts.

## 2026-09-21

### Added
- Awesome Ratatui app browser in the app store (#9).
- Navigation bar position toggle, top or bottom (#10).

## 2026-08-27

### Added
- App store, with install, uninstall and a registered apps list (#8).

## 2026-08-01

### Added
- Settings, with pages for About, Appearance, Audio, Button mapping, Display, Git, Hotkeys, Power, Scripts, Utilities and Wi-Fi/Bluetooth. Settings are persisted (#7).
- File explorer app (#6).

## 2026-07-13

### Added
- Third-party app and script support: `x/build`, `x/install` and `x/setup` scripts, plus updated `x/clone` scripts (#5).

### Changed
- Moved the character set and text editor apps out of `applications/default` (#5).

## 2026-07-12

### Fixed
- Log scrolling (two fixes).

## 2026-07-10

### Added
- Logging utility with an in-app log view (#3).

## 2026-07-01

### Added
- Restart and shut down actions (#2).

## 2026-06-30

### Added
- On-screen keyboard and a text editor app, with a later interface update.
- Third-party dashboard support, including input handling for third-party dashboards and applications (#1).
- `x/clone.sh` and `x/clone.bat` clone scripts.

## 2026-06-29

### Added
- Initial TUIX release: window/app manager with process manager and registry, character set app, two sample dashboards, and settings and touchscreen page stubs.
