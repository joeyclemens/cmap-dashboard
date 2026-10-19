//! Reads the cmap export (CSV, or an already-converted .xlsx) and turns it into
//! one row per person/project with a value per week column.
//! This is a port of `prepare_unpivoted_data` from the Python app.
use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug)]
pub enum Cell {
    Empty,
    Text(String),
    Num(f64),
    Date(NaiveDate),
}

impl Cell {
    fn text(&self) -> String {
        match self {
            Cell::Text(s) => s.trim().to_string(),
            _ => String::new(),
        }
    }
    fn number(&self) -> Option<f64> {
        match self {
            Cell::Num(n) => Some(*n),
            Cell::Text(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }
}

pub fn read_grid(path: &Path) -> Result<Vec<Vec<Cell>>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "csv" => read_csv_grid(path),
        "xlsx" | "xlsm" | "xls" => read_xlsx_grid(path),
        other => bail!("Unsupported input type .{other} ({})", path.display()),
    }
}

fn read_csv_grid(path: &Path) -> Result<Vec<Vec<Cell>>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes);
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        rows.push(
            rec.iter()
                .map(|f| {
                    if f.trim().is_empty() {
                        Cell::Empty
                    } else {
                        Cell::Text(f.to_string())
                    }
                })
                .collect(),
        );
    }
    Ok(rows)
}

fn read_xlsx_grid(path: &Path) -> Result<Vec<Vec<Cell>>> {
    use calamine::{open_workbook_auto, Data, Reader};
    let mut wb = open_workbook_auto(path).with_context(|| format!("opening {}", path.display()))?;
    let name = wb
        .sheet_names()
        .first()
        .cloned()
        .context("workbook has no sheets")?;
    let range = wb.worksheet_range(&name)?;
    Ok(range
        .rows()
        .map(|r| {
            r.iter()
                .map(|c| match c {
                    Data::Empty => Cell::Empty,
                    Data::String(s) if s.trim().is_empty() => Cell::Empty,
                    Data::String(s) => Cell::Text(s.clone()),
                    Data::Float(f) => Cell::Num(*f),
                    Data::Int(i) => Cell::Num(*i as f64),
                    Data::DateTime(dt) => dt
                        .as_datetime()
                        .map(|d| Cell::Date(d.date()))
                        .unwrap_or(Cell::Empty),
                    Data::DateTimeIso(s) => Cell::Text(s.clone()),
                    _ => Cell::Empty,
                })
                .collect()
        })
        .collect())
}

fn parse_date_cell(c: &Cell) -> Option<NaiveDate> {
    match c {
        Cell::Date(d) => Some(*d),
        Cell::Num(n) if *n > 30000.0 && *n < 80000.0 => {
            NaiveDate::from_ymd_opt(1899, 12, 30)?
                .checked_add_signed(chrono::Duration::days(*n as i64))
        }
        Cell::Text(s) => {
            let s = s.trim();
            for f in [
                "%d %b %y", "%d %b %Y", "%Y-%m-%d", "%d/%m/%Y", "%d/%m/%y", "%d-%b-%y", "%d-%b-%Y",
                "%Y-%m-%dT%H:%M:%S",
            ] {
                if let Ok(d) = NaiveDate::parse_from_str(s, f) {
                    return Some(d);
                }
            }
            None
        }
        _ => None,
    }
}

/// person -> department (stripped); None when the department cell is blank.
pub fn load_roles(path: &Path) -> Result<HashMap<String, Option<String>>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    let mut rdr = csv::Reader::from_reader(bytes);
    let headers = rdr.headers()?.clone();
    let col = |n: &str| {
        headers
            .iter()
            .position(|h| h.trim() == n)
            .with_context(|| format!("{} is missing a '{n}' column", path.display()))
    };
    let (pi, di) = (col("Person")?, col("Department")?);
    let mut map = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let person = rec.get(pi).unwrap_or("").to_string();
        if person.is_empty() {
            continue;
        }
        let dept = rec.get(di).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        map.insert(person, dept);
    }
    Ok(map)
}

pub fn load_projects(path: &Path) -> Result<Vec<String>> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    let mut rdr = csv::Reader::from_reader(bytes);
    let headers = rdr.headers()?.clone();
    let ti = headers
        .iter()
        .position(|h| h.trim() == "Project Title")
        .with_context(|| format!("{} is missing a 'Project Title' column", path.display()))?;
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        if let Some(t) = rec.get(ti) {
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct PRow {
    pub person: String,
    pub dept: String,
    pub project: Option<String>,
    /// One value per entry in `Prepared::dates`.
    pub values: Vec<Option<f64>>,
}

pub struct Prepared {
    /// Week-start dates, in column order.
    pub dates: Vec<NaiveDate>,
    pub rows: Vec<PRow>,
    /// Truncated cmap project names (ending in "...") with no matching entry in project.csv.
    pub unmatched: Vec<String>,
}

fn project_department_override(project: &str) -> Option<&'static str> {
    let n = project.trim().to_uppercase();
    if !n.contains("ENGLANDNHP") {
        return None;
    }
    if n.contains(" - EP") {
        return Some("Equipment & Technology");
    }
    if n.contains(" - HP") {
        return Some("Healthcare Planning");
    }
    None
}

pub fn prepare(
    grid: &[Vec<Cell>],
    roles: &HashMap<String, Option<String>>,
    projects: &[String],
) -> Result<Prepared> {
    let header = grid.first().context("input file is empty")?;
    let date_cols: Vec<(usize, NaiveDate)> = header
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(i, c)| parse_date_cell(c).map(|d| (i, d)))
        .collect();
    if date_cols.is_empty() {
        bail!("No date columns found in the header row");
    }

    let exact: HashSet<&str> = projects.iter().map(|s| s.as_str()).collect();
    let mut current: Option<String> = None;
    let mut processed: HashSet<String> = HashSet::new();
    let mut rows = Vec::new();
    let mut unmatched: Vec<String> = Vec::new();

    for r in &grid[1..] {
        let first = r.first().map(|c| c.text()).unwrap_or_default();
        let mut project: Option<String> = None;

        if roles.contains_key(&first) {
            if current.is_none() || !processed.contains(&first) {
                // Person heading row: switches person and is not data itself.
                current = Some(first.clone());
                processed.insert(first);
                continue;
            }
        } else if let Some(cp) = &current {
            if !first.is_empty() && !first.contains(cp.as_str()) {
                project = Some(first.clone());
            }
        }

        let Some(person) = current.clone() else { continue };
        let dept = roles
            .get(&person)
            .cloned()
            .flatten()
            .unwrap_or_default();
        if dept == "Ignore" {
            continue;
        }

        let values: Vec<Option<f64>> = date_cols
            .iter()
            .map(|(i, _)| r.get(*i).and_then(|c| c.number()))
            .collect();

        // Match the (often truncated) cmap project name to project.csv.
        let mut matched = false;
        if let Some(p) = project.clone() {
            if exact.contains(p.as_str()) {
                matched = true;
            } else {
                let key: String = p.chars().take(12).collect();
                if let Some(full) = projects.iter().find(|t| t.starts_with(&key)) {
                    project = Some(full.clone());
                    matched = true;
                }
            }
            let internal = p == "Time Off" || p == "Internal Codes";
            let has_value = values.iter().flatten().any(|v| v.abs() > 1e-9);
            let truncated = p.ends_with("...") || p.ends_with('…');
            if !matched && truncated && !internal && has_value && !unmatched.contains(&p) {
                unmatched.push(p);
            }
        }

        let dept = project
            .as_deref()
            .and_then(project_department_override)
            .map(|s| s.to_string())
            .unwrap_or(dept);

        rows.push(PRow { person, dept, project, values });
    }

    Ok(Prepared {
        dates: date_cols.iter().map(|(_, d)| *d).collect(),
        rows,
        unmatched,
    })
}

/// Latest available date that is <= `selected` (the Python app's fallback rule).
/// Returns (date, fallback_used).
pub fn resolve_effective_date(dates: &[NaiveDate], selected: NaiveDate) -> (NaiveDate, bool) {
    if dates.contains(&selected) {
        return (selected, false);
    }
    let mut earlier: Vec<NaiveDate> = dates.iter().copied().filter(|d| *d < selected).collect();
    earlier.sort();
    match earlier.last() {
        Some(d) => (*d, true),
        None => (selected, false),
    }
}

pub fn date_from_filename(path: &Path) -> Option<NaiveDate> {
    let name = path.file_name()?.to_str()?;
    let b = name.as_bytes();
    for i in 0..b.len().saturating_sub(9) {
        if !(name.is_char_boundary(i) && name.is_char_boundary(i + 10)) {
            continue;
        }
        let s = &name[i..i + 10];
        if b[i + 4] == b'-' && b[i + 7] == b'-' {
            if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                return Some(d);
            }
        }
    }
    None
}
