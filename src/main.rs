//! cmap-dashboard: download this week's cmap export, compare with last week's,
//! and write the dashboard HTML + PNG for each department.
mod browser;
mod cdp;
mod creds;
mod data;
mod html;
mod summary;

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use std::path::{Path, PathBuf};
use summary::*;

const DEPARTMENTS: [&str; 3] = ["Equipment & Technology", "Healthcare Planning", "Healthcare Design"];

struct Args {
    download: bool,
    headless: bool,
    png: bool,
    lookup: bool,
    forget_login: bool,
    no_pause: bool,
    open: bool,
    csv: Option<PathBuf>,
    previous: Option<PathBuf>,
    date: Option<NaiveDate>,
    data_dir: Option<PathBuf>,
    input_dir: Option<PathBuf>,
    out_dir: PathBuf,
}

const HELP: &str = "cmap-dashboard - weekly cmap resource dashboards in one step

USAGE: cmap-dashboard [options]

  (no options)         download from cmap, find last week's file, build dashboards
  --no-download        skip cmap; use the newest CSV/XLSX in the input folder
  --csv <file>         use this export as the current week (.csv or .xlsx)
  --previous <file>    use this export as last week (default: newest earlier file)
  --date <YYYY-MM-DD>  week-start to report on (default: date in the file name)
  --headless           run the cmap browser hidden (default: visible)
  --no-png             write HTML only, skip PNG export
  --no-lookup          do not look up truncated project names in cmap
  --forget-login       clear the saved cmap login and ask for it again
  --no-pause           do not wait for Enter at the end (for scheduled runs)
  --no-open            do not open the output folder when finished
  --input-dir <dir>    where exports live (default: OUT_DIR from .env, else Downloads\\CMAP)
  --data-dir <dir>     folder holding role.csv and project.csv (default: .\\data)
  --out-dir <dir>      where dashboards are written (default: .\\output)
";

fn parse_args() -> Result<Args> {
    let mut a = Args {
        download: true,
        headless: false,
        png: true,
        lookup: true,
        forget_login: false,
        no_pause: false,
        open: true,
        csv: None,
        previous: None,
        date: None,
        data_dir: None,
        input_dir: None,
        out_dir: PathBuf::from("output"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--no-download" => a.download = false,
            "--headless" => a.headless = true,
            "--no-png" => a.png = false,
            "--no-lookup" => a.lookup = false,
            "--forget-login" => a.forget_login = true,
            "--no-pause" => a.no_pause = true,
            "--no-open" => a.open = false,
            "--csv" => {
                a.csv = Some(PathBuf::from(val("--csv")?));
                a.download = false;
            }
            "--previous" => a.previous = Some(PathBuf::from(val("--previous")?)),
            "--date" => {
                a.date = Some(
                    NaiveDate::parse_from_str(&val("--date")?, "%Y-%m-%d")
                        .context("--date must look like 2026-10-19")?,
                )
            }
            "--input-dir" => a.input_dir = Some(PathBuf::from(val("--input-dir")?)),
            "--data-dir" => a.data_dir = Some(PathBuf::from(val("--data-dir")?)),
            "--out-dir" => a.out_dir = PathBuf::from(val("--out-dir")?),
            other => bail!("Unknown option {other}\n\n{HELP}"),
        }
    }
    Ok(a)
}

/// When run as a shared .exe (not from cargo's target folder), work from the exe's own folder
/// so data\, output\ and the saved cmap session live next to it wherever it was launched from.
fn use_exe_folder_as_home() {
    if let Ok(exe) = std::env::current_exe() {
        let in_target = exe.components().any(|c| c.as_os_str() == "target");
        if !in_target {
            if let Some(dir) = exe.parent() {
                let _ = std::env::set_current_dir(dir);
            }
        }
    }
}

fn load_env() {
    if dotenvy::dotenv().is_ok() {
        return;
    }
    // Fall back to the original downloader's .env so credentials are not duplicated.
    for p in ["../auto weekly cmap/.env", "auto weekly cmap/.env"] {
        if dotenvy::from_path(p).is_ok() {
            return;
        }
    }
}

fn input_dir(args: &Args) -> PathBuf {
    args.input_dir
        .clone()
        .or_else(|| std::env::var_os("OUT_DIR").map(PathBuf::from))
        .or_else(|| dirs::download_dir().map(|d| d.join("CMAP")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn data_dir(args: &Args) -> Result<PathBuf> {
    let mut candidates = vec![];
    if let Some(d) = &args.data_dir {
        candidates.push(d.clone());
    }
    candidates.push(PathBuf::from("data"));
    if let Ok(exe) = std::env::current_exe() {
        for anc in exe.ancestors().skip(1).take(4) {
            candidates.push(anc.join("data"));
        }
    }
    candidates
        .into_iter()
        .find(|d| d.join("role.csv").exists() && d.join("project.csv").exists())
        .context("Could not find role.csv and project.csv (use --data-dir)")
}

/// Exports in `dir`, newest first, one per date (CSV preferred over XLSX).
fn list_exports(dir: &Path) -> Vec<(NaiveDate, PathBuf)> {
    let mut by_date: std::collections::BTreeMap<NaiveDate, PathBuf> = Default::default();
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    for e in rd.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
        if ext != "csv" && ext != "xlsx" {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("~$") || name.contains("(1)") {
            continue;
        }
        if let Some(d) = data::date_from_filename(&p) {
            let keep_new = match by_date.get(&d) {
                None => true,
                Some(old) => ext == "csv" && old.extension().and_then(|x| x.to_str()) != Some("csv"),
            };
            if keep_new {
                by_date.insert(d, p);
            }
        }
    }
    let mut v: Vec<_> = by_date.into_iter().collect();
    v.reverse();
    v
}

fn records_for(prep: &data::Prepared, date: NaiveDate, dept: &str) -> Vec<Rec> {
    let Some(col) = prep.dates.iter().position(|d| *d == date) else { return vec![] };
    prep.rows
        .iter()
        .filter(|r| r.dept.trim() == dept.trim())
        .filter(|r| !matches!(r.project.as_deref(), Some("Time Off") | Some("Internal Codes")))
        .filter_map(|r| {
            r.values[col].map(|v| Rec {
                person: r.person.clone(),
                project: r.project.clone(),
                resource: v,
            })
        })
        .collect()
}

fn load(path: &Path, data: &Path) -> Result<data::Prepared> {
    let roles = data::load_roles(&data.join("role.csv"))?;
    let projects = data::load_projects(&data.join("project.csv"))?;
    let grid = data::read_grid(path)?;
    data::prepare(&grid, &roles, &projects).with_context(|| format!("processing {}", path.display()))
}


fn load_previous(
    path: Option<&Path>,
    data: &Path,
    date: NaiveDate,
) -> Result<Option<(data::Prepared, NaiveDate)>> {
    let Some(p) = path else {
        println!("[app] No earlier export found - comparisons will be switched off");
        return Ok(None);
    };
    println!("[app] Previous week: {}", p.display());
    let prep = load(p, data)?;
    let sel = data::date_from_filename(p)
        .or_else(|| prep.dates.iter().copied().filter(|d| *d < date).max())
        .or_else(|| prep.dates.first().copied());
    Ok(sel.map(|s| {
        let (d, _) = data::resolve_effective_date(&prep.dates, s);
        (prep, d)
    }))
}

/// Appends full project names to project.csv (Private left blank). Returns how many were new.
#[allow(dead_code)]
fn append_projects(path: &Path, names: Vec<String>) -> Result<usize> {
    let existing = data::load_projects(path)?;
    let mut text = std::fs::read_to_string(path)?;
    if !text.ends_with('\n') {
        text.push_str("\r\n");
    }
    let mut added = 0;
    for n in names {
        if existing.contains(&n) {
            continue;
        }
        let field = if n.contains(',') || n.contains('"') {
            format!("\"{}\"", n.replace('"', "\"\""))
        } else {
            n.clone()
        };
        text.push_str(&format!("{field},\r\n"));
        added += 1;
    }
    if added > 0 {
        std::fs::write(path, text)?;
    }
    Ok(added)
}

#[tokio::main]
async fn main() {
    let no_pause = std::env::args().any(|a| a == "--no-pause");
    let result = run().await;
    if let Err(e) = &result {
        eprintln!("\nError: {e:#}");
        if format!("{e:#}").to_lowercase().contains("cmap") {
            eprintln!("If your cmap login was wrong, run again with --forget-login to re-enter it.");
        }
    }
    // A double-clicked .exe closes its window on exit; hold it open so the output can be read.
    let from_cargo = std::env::current_exe()
        .map(|p| p.components().any(|c| c.as_os_str() == "target"))
        .unwrap_or(false);
    if !from_cargo && !no_pause {
        println!("\nPress Enter to close...");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(if result.is_err() { 1 } else { 0 });
}

async fn run() -> Result<()> {
    let args = parse_args()?;
    use_exe_folder_as_home();
    load_env();
    if args.forget_login {
        creds::forget();
    }
    let data = data_dir(&args)?;
    let in_dir = input_dir(&args);

    // 1. Current week's export
    let current_path = if let Some(p) = &args.csv {
        p.clone()
    } else if args.download {
        println!("[app] Downloading this week's export from cmap...");
        creds::ensure_credentials()?;
        cdp::download_csv(args.headless, in_dir.clone()).await?
    } else {
        list_exports(&in_dir)
            .into_iter()
            .next()
            .map(|x| x.1)
            .with_context(|| format!("No exports found in {}", in_dir.display()))?
    };
    println!("[app] Current week : {}", current_path.display());
    let current = load(&current_path, &data)?;

    let file_date = data::date_from_filename(&current_path);
    let selected = args
        .date
        .or(file_date)
        .or_else(|| current.dates.first().copied())
        .context("Could not work out which week to report on")?;
    let (mut date, fallback) = data::resolve_effective_date(&current.dates, selected);
    if !current.dates.contains(&date) {
        date = current.dates[0];
        println!("[app] {selected} is not in the file; using first week column {date}");
    } else if fallback {
        println!("[app] No column for {selected}; using the latest earlier week {date}");
    }

    // 2. Last week's export (the newest earlier file unless --previous is given)
    let previous_path = args.previous.clone().or_else(|| {
        let cur_date = file_date.unwrap_or(date);
        list_exports(&in_dir)
            .into_iter()
            .find(|(d, p)| *d < cur_date && p != &current_path)
            .map(|x| x.1)
    });
    #[allow(unused_mut)]
    let mut previous = load_previous(previous_path.as_deref(), &data, date)?;
    #[allow(unused_mut)]
    let mut current = current;

    // Fill in full names for truncated projects missing from project.csv.
    let mut pending: Vec<String> = current.unmatched.clone();
    if let Some((p, _)) = &previous {
        for u in &p.unmatched {
            if !pending.contains(u) {
                pending.push(u.clone());
            }
        }
    }
    if args.lookup && !pending.is_empty() {
        {
            creds::ensure_credentials()?;
            println!("[app] Looking up {} truncated project name(s) in cmap...", pending.len());
            match cdp::lookup_project_names(args.headless, &pending).await {
                Ok(found) if !found.is_empty() => {
                    let added = append_projects(&data.join("project.csv"), found.values().cloned().collect())?;
                    println!("[app] Added {added} project(s) to project.csv");
                    current = load(&current_path, &data)?;
                    previous = load_previous(previous_path.as_deref(), &data, date)?;
                }
                Ok(_) => {}
                Err(e) => println!("[warn] Project name lookup failed: {e:#}"),
            }
        }
    }

    if !current.unmatched.is_empty() {
        println!("\n[warn] Truncated project names still not in project.csv - add the full name from cmap:");
        for u in &current.unmatched {
            println!("         - {u}");
        }
        println!();
    }

    // 3. Build every department
    std::fs::create_dir_all(&args.out_dir)?;
    let mut labels: Vec<&str> = html::FUN_LABELS.to_vec();
    fastrand::shuffle(&mut labels);
    let mut png_jobs = Vec::new();
    let date_str = date.format("%Y-%m-%d").to_string();
    let mut n = 0usize;
    for dept in DEPARTMENTS {
        let recs = records_for(&current, date, dept);
        if recs.is_empty() {
            println!("[app] {dept}: no data for {date_str}, skipped");
            continue;
        }
        let cur_sum = summarize(&recs, dept);
        let prev = previous.as_ref().and_then(|(prep, d)| {
            let r = records_for(prep, *d, dept);
            if r.is_empty() { None } else { Some((summarize(&r, dept), *d)) }
        });
        let cmp = build_comparison(&cur_sum, prev.as_ref().map(|p| &p.0));
        let brief = build_brief_text(&cur_sum, prev.as_ref().map(|p| &p.0));
        let prev_label = prev.as_ref().map(|p| format_week_label(p.1));
        let page = html::dashboard_html(
            dept,
            date,
            &cur_sum,
            &cmp,
            &brief,
            prev_label.as_deref(),
            labels[n % labels.len()],
        );
        n += 1;
        let stem = format!("{}_{}", sanitize_for_filename(dept), date_str);
        let html_path = args.out_dir.join(format!("{stem}_dashboard.html"));
        std::fs::write(&html_path, page)?;
        println!(
            "[app] {dept}: {} projects, {:.1} days  ->  {}",
            cur_sum.active_projects,
            cur_sum.days_allocated,
            html_path.display()
        );
        png_jobs.push((html_path, args.out_dir.join(format!("{stem}_combined.png"))));
    }
    if png_jobs.is_empty() {
        bail!("No data found for {date_str}");
    }

    // 4. PNGs
    if args.png {
        cdp::render_pngs(&png_jobs).await?;
    }

    println!("\n[app] Done. Output in {}", args.out_dir.display());
    #[cfg(windows)]
    if args.open {
        let _ = std::process::Command::new("explorer").arg(&args.out_dir).spawn();
    }
    #[cfg(not(windows))]
    let _ = args.open;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_new_projects_only() {
        let dir = std::env::temp_dir().join("cmapdash_test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("project.csv");
        std::fs::write(&f, "\u{feff}Project Title,Private\r\nA1 - One,No\r\nA2 - Two,Yes").unwrap();
        let n = append_projects(
            &f,
            vec!["A1 - One".into(), "A3 - Three, Four".into(), "A4 - Four".into()],
        )
        .unwrap();
        assert_eq!(n, 2);
        let titles = data::load_projects(&f).unwrap();
        assert_eq!(titles, vec!["A1 - One", "A2 - Two", "A3 - Three, Four", "A4 - Four"]);
    }
}
