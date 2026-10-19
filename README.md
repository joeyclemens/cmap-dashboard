# cmap-dashboard

One command replaces the whole weekly routine:

1. logs in to cmap (using Chrome or Edge) and exports the Resource Availability CSV (the old `auto weekly cmap` app)
2. reads the CSV directly (no manual Excel conversion)
3. finds last week's export automatically
4. writes the three department dashboards as HTML and a 3840x2160 PNG (the old `app_v2.py`)
5. opens the output folder

Double-click `run.bat` (or run it from a terminal with the options below).

## Options
    run.bat --no-download           use the newest export already in Downloads\CMAP
    run.bat --csv "file.csv"        use a specific export (.csv or .xlsx)
    run.bat --previous "old.csv"    force which file counts as last week
    run.bat --date 2026-10-19       report on a specific week
    run.bat --headless              hide the cmap browser window
    run.bat --no-png                HTML only
    run.bat --forget-login          clear the saved cmap login and re-enter it
    run.bat --no-lookup             do not look up truncated project names in cmap
    run.bat --help

## Files
- `data/role.csv`, `data/project.csv` - same lists as the Python app. Update them here.
- Credentials come from `.env` (CMAP_USER / CMAP_PASS). If there is no `.env` here, the one in
  `..\auto weekly cmap` is used. `OUT_DIR` in that file decides where exports are saved
  (default `Downloads\CMAP`).
- Output goes to `output\` as `<Department>_<date>_dashboard.html` and `_combined.png`.

## Notes
- cmap truncates some project names (ending in `...`). When one is not in `project.csv`, the app
  opens cmap's Projects tab, types the project number into the 'Project No - Title' filter, and
  appends the full name to `data\project.csv` (Private left blank). Anything it cannot resolve is
  listed as a warning so you can add it by hand. Debug snapshots go to `.cmap_browser\debug`.
- The browser steps drive your installed Chrome (or Edge) directly over the DevTools protocol, so there is
  no Playwright, driver or browser download. A dedicated profile for the cmap session is kept in
  `.cmap_browser` next to the app. Set `CMAP_BROWSER` to a browser .exe to override which one is used.

## Sharing
`build_release.bat` makes `dist\cmap-dashboard.zip` (exe + data + instructions, no login details).
Each colleague is asked for their own cmap login on first run; it is kept in Windows Credential Manager.

## Setting up from a fresh clone
The real `data/role.csv` and `data/project.csv` (staff and project names) are not in this repo.
Copy the examples and replace the contents with your own:

    copy data\role.example.csv data\role.csv
    copy data\project.example.csv data\project.csv

`role.csv` needs `Role,Person,Department` columns; `project.csv` needs `Project Title,Private`.
