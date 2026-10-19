//! Per-department summaries, week-on-week comparison and the "this week in brief" text.
//! Port of summarize_department_data / build_comparison / build_brief_text.
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Rec {
    pub person: String,
    pub project: Option<String>,
    pub resource: f64,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub active_projects: usize,
    pub days_allocated: f64,
    pub project_grouped: Vec<(String, f64)>,
    pub project_total: f64,
    pub top_project_name: String,
    pub top_project_pct: f64,
    #[allow(dead_code)]
    pub hidden_projects: Vec<String>,
    pub top_person_name: String,
    pub top_person_value: f64,
    pub top_people: Vec<(String, f64)>,
    pub remaining_people: usize,
    pub project_people: HashMap<String, Vec<(String, f64)>>,
    pub person_projects: HashMap<String, Vec<(String, f64)>>,
    pub other_projects: Vec<(String, f64)>,
    pub person_totals: HashMap<String, f64>,
}

pub struct Comparison {
    pub previous_person_totals: Option<HashMap<String, f64>>,
    pub active_delta_text: String,
    pub active_delta_colour: Delta,
    pub days_delta_text: String,
    pub days_delta_colour: Delta,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Delta {
    Pos,
    Neg,
    Neutral,
}

// ---------------------------------------------------------------- text helpers

pub fn html_escape(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Python's textwrap.shorten (word based).
pub fn shorten(text: &str, width: usize, placeholder: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let joined = words.join(" ");
    if joined.chars().count() <= width {
        return joined;
    }
    let ph = placeholder.chars().count();
    let mut out = String::new();
    for w in &words {
        let candidate = if out.is_empty() {
            w.to_string()
        } else {
            format!("{out} {w}")
        };
        if candidate.chars().count() + ph > width {
            break;
        }
        out = candidate;
    }
    if out.is_empty() {
        placeholder.trim().to_string()
    } else {
        format!("{out}{placeholder}")
    }
}

pub fn short_project_name(name: &str) -> String {
    let value = name.trim();
    if let Some((first, rest)) = value.split_once(" - ") {
        if first.chars().any(|c| c.is_ascii_digit()) {
            return rest.trim().to_string();
        }
    }
    value.to_string()
}

const CLEAN_REPLACEMENTS: [(&str, &str); 12] = [
    (" - EP TO#4", ""),
    (" - EP TO#3", ""),
    (" - EP TO#2", ""),
    (" - EP TO#1", ""),
    (" - EP", ""),
    (" TO#4", ""),
    (" TO#3", ""),
    (" TO#2", ""),
    (" TO#1", ""),
    (" RIBA Stage 4b Prolongation+", " RIBA"),
    (" RIBA Stage 4", " RIBA"),
    (" RIBA Stage 3", " RIBA"),
];

pub fn full_clean_project_name(name: &str) -> String {
    let mut n = short_project_name(name);
    for (old, new) in CLEAN_REPLACEMENTS {
        n = n.replace(old, new);
    }
    n.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn cleaner_project_name(name: &str, width: usize) -> String {
    shorten(&full_clean_project_name(name), width, "...")
}

pub fn compact_project_name(name: &str, width: usize) -> String {
    shorten(&short_project_name(name), width, "…")
}

pub fn fmt_days(v: f64) -> String {
    let s = format!("{v:.1}");
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    format!("{s}d")
}

pub fn format_week_label(d: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    format!("{} {}", d.day(), d.format("%b %Y"))
}

pub fn sanitize_for_filename(v: &str) -> String {
    v.replace(' ', "_").replace('&', "and").replace('/', "-")
}

// ------------------------------------------------------------------ summaries

fn sort_desc(v: &mut Vec<(String, f64)>) {
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
}

fn project_grouped_data(recs: &[Rec], dept: &str, limit: usize) -> (Vec<(String, f64)>, Vec<String>) {
    let mut m: BTreeMap<String, f64> = BTreeMap::new();
    for r in recs {
        if let Some(p) = &r.project {
            *m.entry(p.clone()).or_default() += r.resource;
        }
    }
    let mut g: Vec<(String, f64)> = m.into_iter().collect();
    // Equipment & Technology hides anything under 2% of its total.
    if dept.trim().eq_ignore_ascii_case("equipment & technology") {
        let total: f64 = g.iter().map(|x| x.1).sum();
        if total > 0.0 {
            g.retain(|x| x.1 / total >= 0.02);
        }
    }
    g.retain(|x| x.1 > 0.0);
    sort_desc(&mut g);

    let mut hidden = Vec::new();
    if g.len() > limit {
        hidden = g[limit - 1..].iter().map(|x| x.0.clone()).collect();
        let other_value: f64 = g[limit - 1..].iter().map(|x| x.1).sum();
        let n = g.len() - (limit - 1);
        g.truncate(limit - 1);
        g.push((format!("Other ({n})"), other_value));
    }
    (g, hidden)
}

pub fn summarize(recs: &[Rec], dept: &str) -> Summary {
    let (project_grouped, hidden_projects) = project_grouped_data(recs, dept, 7);
    let project_total: f64 = project_grouped.iter().map(|x| x.1).sum();
    let active_projects = recs
        .iter()
        .filter_map(|r| r.project.as_deref())
        .collect::<HashSet<_>>()
        .len();
    let (top_project_name, top_project_value) = project_grouped
        .first()
        .cloned()
        .unwrap_or(("N/A".to_string(), 0.0));
    let top_project_pct = if project_total > 0.0 {
        top_project_value / project_total * 100.0
    } else {
        0.0
    };

    let mut pg: BTreeMap<String, f64> = BTreeMap::new();
    for r in recs {
        *pg.entry(r.person.clone()).or_default() += r.resource;
    }
    let person_totals: HashMap<String, f64> = pg.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let mut person_grouped: Vec<(String, f64)> = pg.into_iter().collect();
    sort_desc(&mut person_grouped);
    let (top_person_name, top_person_value) = person_grouped
        .first()
        .cloned()
        .unwrap_or(("N/A".to_string(), 0.0));
    const PEOPLE_LIMIT: usize = 8;
    let top_people: Vec<_> = person_grouped.iter().take(PEOPLE_LIMIT).cloned().collect();
    let remaining_people = person_grouped.len().saturating_sub(PEOPLE_LIMIT);

    // Who works on what (positive allocations only).
    let mut by_proj_person: BTreeMap<(String, String), f64> = BTreeMap::new();
    let mut by_person_proj: BTreeMap<(String, String), f64> = BTreeMap::new();
    for r in recs.iter().filter(|r| r.resource > 0.0) {
        if let Some(p) = &r.project {
            *by_proj_person.entry((p.clone(), r.person.clone())).or_default() += r.resource;
            *by_person_proj.entry((r.person.clone(), p.clone())).or_default() += r.resource;
        }
    }
    let mut project_people: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for ((p, person), d) in &by_proj_person {
        project_people.entry(p.clone()).or_default().push((person.clone(), *d));
    }
    for v in project_people.values_mut() {
        sort_desc(v);
    }
    let mut person_projects: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for ((person, p), d) in &by_person_proj {
        person_projects.entry(person.clone()).or_default().push((p.clone(), *d));
    }
    for v in person_projects.values_mut() {
        sort_desc(v);
    }

    // The "Other (n)" tile merges several projects.
    let mut other_projects: Vec<(String, f64)> = Vec::new();
    let other_name = project_grouped
        .iter()
        .find(|x| x.0.starts_with("Other ("))
        .map(|x| x.0.clone());
    if let (false, Some(other_name)) = (hidden_projects.is_empty(), other_name) {
        let mut merged: Vec<(String, f64)> = Vec::new();
        for proj in &hidden_projects {
            let days: f64 = recs
                .iter()
                .filter(|r| r.resource > 0.0 && r.project.as_deref() == Some(proj.as_str()))
                .map(|r| r.resource)
                .sum();
            other_projects.push((proj.clone(), days));
            for (person, d) in project_people.get(proj).cloned().unwrap_or_default() {
                match merged.iter_mut().find(|m| m.0 == person) {
                    Some(m) => m.1 += d,
                    None => merged.push((person, d)),
                }
            }
        }
        sort_desc(&mut merged);
        project_people.insert(other_name, merged);
        sort_desc(&mut other_projects);
    }

    Summary {
        active_projects,
        days_allocated: recs.iter().map(|r| r.resource).sum(),
        project_grouped,
        project_total,
        top_project_name,
        top_project_pct,
        hidden_projects,
        top_person_name,
        top_person_value,
        top_people,
        remaining_people,
        project_people,
        person_projects,
        other_projects,
        person_totals,
    }
}

fn format_delta(cur: f64, prev: Option<f64>, suffix: &str, integer: bool) -> (String, Delta) {
    let Some(prev) = prev else {
        return ("No previous week".to_string(), Delta::Neutral);
    };
    let d = cur - prev;
    if d.abs() < 1e-9 {
        return (format!("0{suffix} vs last week"), Delta::Neutral);
    }
    let arrow = if d > 0.0 { '↑' } else { '↓' };
    let colour = if d > 0.0 { Delta::Pos } else { Delta::Neg };
    let text = if integer {
        format!("{}{suffix}", d.abs() as i64)
    } else {
        format!("{:.1}{suffix}", d.abs())
    };
    (format!("{arrow} {text} vs last week"), colour)
}

pub fn build_comparison(cur: &Summary, prev: Option<&Summary>) -> Comparison {
    let (at, ac) = format_delta(
        cur.active_projects as f64,
        prev.map(|p| p.active_projects as f64),
        "",
        true,
    );
    let (dt, dc) = format_delta(cur.days_allocated, prev.map(|p| p.days_allocated), "d", false);
    Comparison {
        previous_person_totals: prev.map(|p| p.person_totals.clone()),
        active_delta_text: at,
        active_delta_colour: ac,
        days_delta_text: dt,
        days_delta_colour: dc,
    }
}

pub fn build_brief_text(cur: &Summary, prev: Option<&Summary>) -> String {
    let first = full_clean_project_name(&cur.top_project_name);
    let lead = format!(
        "{first} is the main focus this week, using {:.0}% of charted resource.",
        cur.top_project_pct
    );
    let names: Vec<&String> = cur.project_grouped.iter().map(|x| &x.0).collect();
    let second = if names.len() >= 3 {
        let top3: f64 = cur.project_grouped.iter().take(3).map(|x| x.1).sum();
        let share = if cur.project_total > 0.0 { top3 / cur.project_total * 100.0 } else { 0.0 };
        format!(
            "With {} and {}, the top three projects account for {:.0}% of the workload.",
            full_clean_project_name(names[1]),
            full_clean_project_name(names[2]),
            share
        )
    } else {
        format!("{} projects are active this week.", cur.active_projects)
    };
    let third = if let Some(prev) = prev {
        let set = |s: &Summary| -> HashSet<String> {
            s.project_grouped
                .iter()
                .filter(|x| !x.0.starts_with("Other ("))
                .map(|x| short_project_name(&x.0))
                .collect()
        };
        let (c, p) = (set(cur), set(prev));
        let mut new: Vec<&String> = c.difference(&p).collect();
        let mut dropped: Vec<&String> = p.difference(&c).collect();
        new.sort();
        dropped.sort();
        if let Some(n) = new.first() {
            format!("New this week: {}.", full_clean_project_name(n))
        } else if let Some(d) = dropped.first() {
            format!("Out of the leading group this week: {}.", full_clean_project_name(d))
        } else {
            "The project mix is broadly stable versus last week.".to_string()
        }
    } else {
        "No previous week was found, so there is no comparison note.".to_string()
    };
    [lead, second, third].join(" ")
}
