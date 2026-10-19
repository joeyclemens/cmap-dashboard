//! Builds the 1920x1080 dashboard page. Port of create_dashboard_html.
use crate::summary::*;
use std::collections::{BTreeMap, HashMap};

pub const EXPORT_WIDTH: u32 = 1920;
pub const EXPORT_HEIGHT: u32 = 1080;
const PALETTE: [&str; 7] = ["#182643", "#253e5c", "#003d36", "#718890", "#52748d", "#8cc3e1", "#28001d"];
const STYLE: &str = include_str!("style.css");

pub const FUN_LABELS: [&str; 8] = [
    "Most Tortured Soul",
    "Who Is Working the Hardest",
    "Heavy Lifter of the Week",
    "Peak Performer",
    "All-Star Contributor",
    "MVP of the Week",
    "Backbone of the Team",
    "Efficiency Expert",
];

/// Python's round(): halves go to the nearest even number.
fn py_round(x: f64) -> i64 {
    let fl = x.floor();
    let diff = x - fl;
    if (diff - 0.5).abs() < 1e-12 {
        if (fl as i64) % 2 == 0 { fl as i64 } else { fl as i64 + 1 }
    } else {
        x.round() as i64
    }
}

fn is_light(hex: &str) -> bool {
    let h = hex.trim_start_matches('#');
    let p = |i: usize| i64::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64;
    0.299 * p(0) + 0.587 * p(2) + 0.114 * p(4) > 150.0
}

fn display_names(people: &[String]) -> HashMap<String, String> {
    let mut firsts: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for p in people {
        firsts
            .entry(p.split_whitespace().next().unwrap_or("").to_string())
            .or_default()
            .push(p);
    }
    let mut out = HashMap::new();
    for (first, group) in firsts {
        for p in group {
            let parts: Vec<&str> = p.split_whitespace().collect();
            let name = if group_len_one(&first, people) || parts.len() < 2 {
                first.clone()
            } else {
                format!("{first} {}.", parts[parts.len() - 1].chars().next().unwrap_or(' '))
            };
            out.insert(p.clone(), name);
        }
    }
    out
}

fn group_len_one(first: &str, people: &[String]) -> bool {
    people
        .iter()
        .filter(|p| p.split_whitespace().next().unwrap_or("") == first)
        .count()
        == 1
}

fn chips(people: &[(String, f64)], names: &HashMap<String, String>, max: usize) -> String {
    if people.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (p, d) in people.iter().take(max) {
        let n = names.get(p).unwrap_or(p);
        out.push_str(&format!(
            "<span class=\"chip\"><b>{}</b> {}</span>",
            html_escape(n),
            fmt_days(*d)
        ));
    }
    if people.len() > max {
        out.push_str(&format!("<span class=\"chip more-chip\">+{}</span>", people.len() - max));
    }
    format!("<div class=\"chips\">{out}</div>")
}

fn person_delta(person: &str, cur: f64, prev: &Option<HashMap<String, f64>>) -> String {
    let Some(prev) = prev else { return String::new() };
    let Some(p) = prev.get(person) else {
        return "<span class=\"pdelta new\">new</span>".to_string();
    };
    let d = cur - p;
    if d.abs() < 0.05 {
        return "<span class=\"pdelta flat\">–</span>".to_string();
    }
    let (cls, arrow) = if d > 0.0 { ("up", '↑') } else { ("down", '↓') };
    format!("<span class=\"pdelta {cls}\">{arrow}{:.1}</span>", d.abs())
}

fn delta_class(d: Delta) -> &'static str {
    match d {
        Delta::Pos => "delta-pos",
        Delta::Neg => "delta-neg",
        Delta::Neutral => "",
    }
}

pub fn dashboard_html(
    dept: &str,
    selected_date: chrono::NaiveDate,
    cur: &Summary,
    cmp: &Comparison,
    brief: &str,
    previous_date_label: Option<&str>,
    fun_label: &str,
) -> String {
    let rows = &cur.project_grouped;
    let (top_name, top_res) = rows
        .first()
        .cloned()
        .unwrap_or(("No project data".to_string(), 0.0));
    let support: Vec<&(String, f64)> = rows.iter().skip(1).take(6).collect();
    let total = if cur.project_total != 0.0 { cur.project_total } else { 1.0 };
    let support_sum: f64 = support.iter().map(|r| r.1).sum();
    let remaining_total = if support_sum != 0.0 { support_sum } else { 1.0 };

    let mut people_names: Vec<String> = cur.person_totals.keys().cloned().collect();
    people_names.sort();
    let names = display_names(&people_names);

    let mut support_cards = String::new();
    for (index, row) in support.iter().enumerate() {
        let pct = row.1 / total * 100.0;
        let grow = row.1 / remaining_total;
        let row_span = py_round(1.0 + grow * 4.5).clamp(2, 3);
        let col_span = if index == 0 && grow >= 0.22 { 2 } else { 1 };
        let max_names = if col_span == 1 && row_span == 2 { 3 } else if col_span == 1 { 5 } else { 7 };
        let colour = PALETTE[(index + 1) % PALETTE.len()];
        let light = if is_light(colour) { " light" } else { "" };
        let people = cur.project_people.get(&row.0).cloned().unwrap_or_default();
        let mut sub = String::new();
        if row.0.starts_with("Other (") && !cur.other_projects.is_empty() {
            let parts: Vec<String> = cur
                .other_projects
                .iter()
                .take(3)
                .map(|(p, d)| format!("{} {}", html_escape(&cleaner_project_name(p, 20)), fmt_days(*d)))
                .collect();
            let more = cur.other_projects.len() as i64 - 3;
            let more_s = if more > 0 { format!(" · +{more}") } else { String::new() };
            sub = format!("<div class=\"mini-sub\">{}{more_s}</div>", parts.join(" · "));
        }
        support_cards.push_str(&format!(
            "\n            <div class=\"mini-card{light} color-{}\" style=\"grid-row: span {row_span}; grid-column: span {col_span};\">\n                <div>\n                    <div class=\"mini-name\">{}</div>\n                    {sub}\n                    {}\n                </div>\n                <div class=\"mini-meta\">{:.1}d <span>{:.0}%</span></div>\n            </div>\n        ",
            index % 6 + 1,
            html_escape(&compact_project_name(&row.0, 26)),
            chips(&people, &names, max_names),
            row.1,
            pct
        ));
    }

    let hero_people = cur.project_people.get(&top_name).cloned().unwrap_or_default();
    let mut hero_html = String::new();
    for (p, d) in hero_people.iter().take(10) {
        let lead = hero_people[0].1;
        let w = if lead != 0.0 { d / lead * 100.0 } else { 0.0 };
        hero_html.push_str(&format!(
            "<div class=\"hero-person\"><span>{}</span><span class=\"hp-days\">{}</span><span class=\"hp-bar\"><i style=\"width:{:.0}%\"></i></span></div>",
            html_escape(p),
            fmt_days(*d),
            w
        ));
    }
    if hero_people.len() > 10 {
        hero_html.push_str(&format!("<div class=\"hero-more\">+{} more</div>", hero_people.len() - 10));
    }

    let max_person = cur
        .top_people
        .iter()
        .map(|p| p.1)
        .fold(f64::MIN, f64::max);
    let max_person = if cur.top_people.is_empty() || max_person == 0.0 { 1.0 } else { max_person };
    let mut people_rows = String::new();
    for (index, (person, days)) in cur.top_people.iter().enumerate() {
        let width = days / max_person * 100.0;
        let main_line = match cur.person_projects.get(person) {
            Some(projects) if !projects.is_empty() => {
                let main = cleaner_project_name(&projects[0].0, 22);
                let more = projects.len() - 1;
                format!("{}{}", html_escape(&main), if more > 0 { format!(" +{more}") } else { String::new() })
            }
            _ => String::new(),
        };
        people_rows.push_str(&format!(
            "\n            <div class=\"person-row\">\n                <div class=\"person-main\">\n                    <div class=\"person-name\">{}</div>\n                    <div class=\"person-proj\">{main_line}</div>\n                </div>\n                <div class=\"person-value\">{:.1}d {}</div>\n                <div class=\"person-bar\"><span class=\"fill color-{}\" style=\"width:{:.1}%\"></span></div>\n            </div>\n        ",
            html_escape(person),
            days,
            person_delta(person, *days, &cmp.previous_person_totals),
            index % 6 + 1,
            width
        ));
    }

    let top_pct = top_res / total * 100.0;
    let hero_basis = (34.0 + top_pct * 0.28).min(56.0).max(40.0);
    let previous_note = match previous_date_label {
        Some(l) => format!("vs uploaded previous week ({})", html_escape(l)),
        None => "No previous week found, so week-on-week comparisons are switched off".to_string(),
    };
    let more_people = if cur.remaining_people > 0 {
        format!("<div class='more'>+{} more</div>", cur.remaining_people)
    } else {
        String::new()
    };

    let style = STYLE
        .replace("__W__", &EXPORT_WIDTH.to_string())
        .replace("__H__", &EXPORT_HEIGHT.to_string())
        .replace("__HERO__", &format!("{hero_basis:.1}"));

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{title} - weekly resource</title>
  <style>
{style}  </style>
</head>
<body>
  <div class="page">
    <div class="subtitle">Week of {week} &nbsp; | &nbsp; {active} active projects &nbsp; | &nbsp; {days:.1} days allocated</div>
    <div class="subnote">{previous_note}</div>

    <div class="metrics">
      <div class="metric">
        <div class="metric-label">Active projects</div>
        <div class="metric-value">{active}</div>
        <div class="metric-sub {ac}">{at}</div>
      </div>
      <div class="metric">
        <div class="metric-label">Days allocated</div>
        <div class="metric-value">{days:.1}</div>
        <div class="metric-sub {dc}">{dt}</div>
      </div>
      <div class="metric">
        <div class="metric-label">Largest project</div>
        <div class="metric-value">{top_pct:.0}%</div>
        <div class="metric-sub">{top_compact}</div>
      </div>
      <div class="metric">
        <div class="metric-label">{fun}</div>
        <div class="metric-value">{top_person}</div>
        <div class="metric-sub">{top_person_days:.1}d allocated</div>
      </div>
    </div>

    <div class="content">
      <div>
        <div class="left-title">RESOURCE BY PROJECT</div>
        <div class="project-stage">
          <div class="hero">
            <div class="hero-top">
              <div class="hero-days">{top_res:.1}d</div>
              <div class="hero-pct">{top_pct:.0}%</div>
            </div>
            <div class="hero-name">{hero_name}</div>
            <div class="hero-people">{hero_html}</div>
            <div class="hero-foot">
              <div>Top project this week &nbsp;·&nbsp; {hero_count} people</div>
            </div>
          </div>
          <div class="mini-grid">
            {support_cards}
          </div>
        </div>
      </div>

      <div class="right-col">
        <div class="panel">
          <div class="panel-title">THIS WEEK IN BRIEF</div>
          <div class="brief-text">{brief}</div>
        </div>
        <div class="panel people-panel">
          <div class="panel-title">DAYS BY PERSON</div>
          {people_rows}
          {more_people}
        </div>
      </div>
    </div>
  </div>
</body>
</html>
"#,
        title = html_escape(dept),
        week = html_escape(&format_week_label(selected_date)),
        active = cur.active_projects,
        days = cur.days_allocated,
        ac = delta_class(cmp.active_delta_colour),
        at = html_escape(&cmp.active_delta_text),
        dc = delta_class(cmp.days_delta_colour),
        dt = html_escape(&cmp.days_delta_text),
        top_compact = html_escape(&compact_project_name(&top_name, 26)),
        fun = html_escape(fun_label),
        top_person = html_escape(&cur.top_person_name),
        top_person_days = cur.top_person_value,
        hero_name = html_escape(&compact_project_name(&top_name, 34)),
        hero_count = hero_people.len(),
        brief = html_escape(brief),
    )
}
