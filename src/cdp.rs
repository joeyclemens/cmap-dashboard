//! Drives the PC's own Chrome/Edge over the DevTools protocol (no Playwright, no driver download).
//! - download_csv: logs in to cmap and exports the Resource Availability CSV
//! - lookup_project_names: finds full project names on the Projects tab
//! - render_pngs: screenshots the dashboard pages
use anyhow::{anyhow, bail, Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser::{SetDownloadBehaviorBehavior, SetDownloadBehaviorParams};
use chromiumoxide::cdp::browser_protocol::emulation::{
    SetDefaultBackgroundColorOverrideParams, SetDeviceMetricsOverrideParams,
};
use chromiumoxide::cdp::browser_protocol::dom::Rgba;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::{Page, ScreenshotParams};
use futures::StreamExt;
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::html::{EXPORT_HEIGHT, EXPORT_WIDTH};

fn report_url() -> String {
    env::var("CMAP_REPORT_URL").unwrap_or_else(|_| {
        "https://mjmedical.cmaphq.com/Reports/Report/PR_ResourceAvailability".to_string()
    })
}

fn profile_dir() -> PathBuf {
    env::var("CMAP_PROFILE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".cmap_browser"))
}

fn debug_dir() -> PathBuf {
    profile_dir().join("debug")
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

// --------------------------------------------------------------------- launch

struct Launched {
    browser: Browser,
    task: tokio::task::JoinHandle<()>,
}

async fn launch(profile: Option<&Path>, headless: bool) -> Result<Launched> {
    let exe = crate::browser::system_browser()
        .context("Could not find Chrome or Edge. Install one of them (or set CMAP_BROWSER to its .exe path).")?;
    let mut cfg = BrowserConfig::builder()
        .chrome_executable(exe)
        .window_size(1400, 1000)
        .arg("--no-first-run")
        .arg("--no-default-browser-check");
    if let Some(p) = profile {
        // Chrome on Windows rejects a relative --user-data-dir ("cannot read and write to its data directory").
        fs::create_dir_all(p)?;
        cfg = cfg.user_data_dir(std::path::absolute(p)?);
    }
    if !headless {
        cfg = cfg.with_head();
    }
    if env::var_os("CMAP_NO_SANDBOX").is_some() {
        cfg = cfg.no_sandbox();
    }
    let (browser, mut handler) = Browser::launch(cfg.build().map_err(|e| anyhow!(e))?)
        .await
        .context("Failed to start Chrome/Edge (is another copy using the same profile?)")?;
    let task = tokio::spawn(async move { while handler.next().await.is_some() {} });
    Ok(Launched { browser, task })
}

impl Launched {
    async fn close(mut self) {
        let _ = self.browser.close().await;
        let _ = self.browser.wait().await;
        self.task.abort();
    }
}

// -------------------------------------------------------------------- helpers

const CM_JS: &str = r#"
(() => {
  if (window.__cm) return true;
  const docs = () => { const out = [document]; for (const f of document.querySelectorAll('iframe')) { try { if (f.contentDocument) out.push(f.contentDocument); } catch (e) {} } return out; };
  const vis = el => { const r = el.getBoundingClientRect(); const s = (el.ownerDocument.defaultView || window).getComputedStyle(el); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
  const ok = el => vis(el) && !el.disabled;
  const txt = el => (el.innerText || el.textContent || '').replace(/\s+/g, ' ').trim();
  const clear = () => docs().forEach(d => d.querySelectorAll('[data-cm]').forEach(e => e.removeAttribute('data-cm')));
  const mark = el => { if (!el) return null; clear(); el.setAttribute('data-cm', '1'); return el.ownerDocument === document ? 'main' : 'iframe'; };
  const match = (t, w, exact) => exact ? t === w : t.includes(w);
  window.__cm = {
    docs, vis, ok, txt, mark, clear,
    find(css, text, exact) {
      for (const d of docs()) for (const el of d.querySelectorAll(css)) {
        if (!ok(el)) continue;
        if (text && !match(txt(el).toLowerCase(), text.toLowerCase(), exact)) continue;
        return el;
      }
      return null;
    },
    leaf(text, exact) {
      const w = text.toLowerCase();
      for (const d of docs()) {
        for (const el of d.querySelectorAll('body *')) {
          if (!ok(el)) continue;
          if (!match(txt(el).toLowerCase(), w, exact)) continue;
          if ([...el.children].some(c => match(txt(c).toLowerCase(), w, exact))) continue;
          return el;
        }
      }
      return null;
    },
    submit(words) {
      for (const d of docs()) for (const el of d.querySelectorAll('button,input[type="submit"],[role="button"]')) {
        if (!ok(el)) continue;
        const t = ((el.tagName === 'INPUT' ? el.value : txt(el)) || '').toLowerCase();
        if (words.some(w => t.includes(w))) return el;
      }
      return null;
    },
    click(sel) { for (const d of docs()) { const e = d.querySelector(sel); if (e) { e.click(); return true; } } return false; },
    selectAll(sel) { for (const d of docs()) { const e = d.querySelector(sel); if (e) { e.focus(); if (e.select) e.select(); return true; } } return false; },
  };
  return true;
})()
"#;

fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

async fn eval_str(page: &Page, js: &str) -> Result<Option<String>> {
    page.evaluate(CM_JS).await?;
    let r = page.evaluate(js).await?;
    Ok(r.into_value::<Option<String>>().unwrap_or(None))
}

async fn eval_bool(page: &Page, js: &str) -> bool {
    if page.evaluate(CM_JS).await.is_err() {
        return false;
    }
    match page.evaluate(js).await {
        Ok(r) => r.into_value::<bool>().unwrap_or(false),
        Err(_) => false,
    }
}

/// Click the element currently marked with data-cm (real mouse click, JS click as a fallback).
async fn click_marked(page: &Page, where_: &str) -> Result<()> {
    if where_ == "main" {
        if let Ok(el) = page.find_element("[data-cm='1']").await {
            if el.click().await.is_ok() {
                return Ok(());
            }
        }
    }
    if eval_bool(page, "__cm.click('[data-cm]')").await {
        return Ok(());
    }
    bail!("could not click the element")
}

async fn wait_idle(page: &Page, timeout_ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        let ready = page
            .evaluate("document.readyState")
            .await
            .ok()
            .and_then(|r| r.into_value::<String>().ok())
            .unwrap_or_default();
        if ready == "complete" {
            break;
        }
        sleep_ms(250).await;
    }
    sleep_ms(600).await;
}

async fn snap(page: &Page, name: &str) {
    let dir = debug_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(bytes) = page
        .screenshot(
            ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .full_page(true)
                .build(),
        )
        .await
    {
        let _ = fs::write(dir.join(format!("{name}.png")), bytes);
    }
    if let Ok(html) = page.content().await {
        let _ = fs::write(dir.join(format!("{name}.html")), html);
    }
}

async fn goto(page: &Page, url: &str) -> Result<()> {
    page.goto(url).await.with_context(|| format!("opening {url}"))?;
    wait_idle(page, 30_000).await;
    Ok(())
}

/// Finds a button-like element by label (button, role=button, title/aria-label, link, any text) and clicks it.
async fn click_button(page: &Page, label: &str) -> Option<String> {
    let l = q(label);
    let strategies: Vec<(String, String)> = vec![
        ("button text".into(), format!("__cm.mark(__cm.find('button', {l}, false))")),
        ("role=button text".into(), format!("__cm.mark(__cm.find('[role=\"button\"]', {l}, false))")),
        (
            "title/aria-label".into(),
            format!(
                "__cm.mark(__cm.find('[title=' + {} + '],[aria-label=' + {} + ']', null, false))",
                q(&format!("\"{label}\"")),
                q(&format!("\"{label}\""))
            ),
        ),
        ("link text".into(), format!("__cm.mark(__cm.find('a', {l}, false))")),
        ("plain text".into(), format!("__cm.mark(__cm.leaf({l}, false))")),
    ];
    for (desc, js) in strategies {
        if let Ok(Some(where_)) = eval_str(page, &js).await {
            if click_marked(page, &where_).await.is_ok() {
                return Some(format!("{desc} ({where_})"));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------- login

async fn type_into(page: &Page, where_: &str, text: &str) -> bool {
    if where_ != "main" {
        return false;
    }
    let Ok(el) = page.find_element("[data-cm='1']").await else { return false };
    let _ = el.click().await;
    let _ = eval_bool(page, "__cm.selectAll('[data-cm]')").await;
    el.type_str(text).await.is_ok()
}

async fn press_on_marked(page: &Page, key: &str) {
    if let Ok(el) = page.find_element("[data-cm='1']").await {
        let _ = el.press_key(key).await;
    }
}

async fn try_login(page: &Page, username: &str, password: &str) -> bool {
    let email_css = r#"input[type="email"],input[name*="email" i],input[name*="user" i],input[name*="login" i]"#;
    let mut email = eval_str(page, &format!("__cm.mark(__cm.find({}, null, false))", q(email_css)))
        .await
        .ok()
        .flatten();
    if email.is_none() {
        email = eval_str(
            page,
            &format!("__cm.mark(__cm.find({}, null, false))", q(r#"input[type="text"],input:not([type])"#)),
        )
        .await
        .ok()
        .flatten();
    }
    let Some(email_where) = email else { return false };
    // Is there a visible password field and a sign-in button too?
    let pw_css = r#"input[type="password"],input[name*="pass" i]"#;
    let has_pw = eval_bool(page, &format!("!!__cm.find({}, null, false)", q(pw_css))).await;
    let has_btn = eval_bool(
        page,
        "!!__cm.submit(['sign in','log in','login','continue'])",
    )
    .await;
    if !has_pw || !has_btn {
        return false;
    }
    println!("[login] found email/password/sign-in; submitting…");
    // re-mark email, type it
    let _ = eval_str(page, &format!("__cm.mark(__cm.find({}, null, false))", q(email_css))).await;
    if eval_str(page, "document.querySelector('[data-cm]') ? 'main' : null").await.ok().flatten().is_none() {
        let _ = eval_str(
            page,
            &format!("__cm.mark(__cm.find({}, null, false))", q(r#"input[type="text"],input:not([type])"#)),
        )
        .await;
    }
    type_into(page, &email_where, username).await;
    let _ = eval_str(page, &format!("__cm.mark(__cm.find({}, null, false))", q(pw_css))).await;
    type_into(page, "main", password).await;
    if let Ok(Some(w)) = eval_str(page, "__cm.mark(__cm.submit(['sign in','log in','login','continue']))").await {
        let _ = click_marked(page, &w).await;
    }
    true
}

// ---------------------------------------------------------------- weeks view

async fn go_to_weeks_view(page: &Page) -> bool {
    println!("[app] trying to switch to 'Weeks' view by clicking…");
    let w = q("Weeks");
    let strategies: Vec<(&str, String)> = vec![
        ("tab role", format!("__cm.mark(__cm.find('[role=\"tab\"]', {w}, false))")),
        ("menuitem role", format!("__cm.mark(__cm.find('[role=\"menuitem\"]', {w}, false))")),
        ("radio role", format!("__cm.mark(__cm.find('[role=\"radio\"]', {w}, false))")),
        ("button text", format!("__cm.mark(__cm.find('button', {w}, false))")),
        (
            "aria-label/title",
            "__cm.mark(__cm.find('[aria-label=\"Weeks\"],[title=\"Weeks\"]', null, false))".to_string(),
        ),
        ("link text", format!("__cm.mark(__cm.find('a', {w}, false))")),
        ("plain text", format!("__cm.mark(__cm.leaf({w}, true))")),
    ];
    for (label, js) in strategies {
        if let Ok(Some(where_)) = eval_str(page, &js).await {
            if click_marked(page, &where_).await.is_ok() {
                println!("[app] clicking 'Weeks' via {label} ({where_})");
                wait_idle(page, 10_000).await;
                snap(page, "04_after_weeks_click").await;
                return true;
            }
        }
    }
    // A <select> with a "Weeks" option
    let sel_js = r#"(() => { for (const d of __cm.docs()) for (const s of d.querySelectorAll('select')) for (const o of s.options) { if ((o.text||'').toLowerCase().includes('weeks')) { s.value = o.value; s.dispatchEvent(new Event('input', {bubbles:true})); s.dispatchEvent(new Event('change', {bubbles:true})); return true; } } return false; })()"#;
    if eval_bool(page, sel_js).await {
        println!("[app] selected 'Weeks' in a <select>");
        wait_idle(page, 10_000).await;
        return true;
    }
    // URL fallback: view=week
    println!("[app] 'Weeks' control not found; trying view=week in the URL");
    let raw = page.url().await.ok().flatten().unwrap_or_default();
    let base = if raw.contains("PR_ResourceAvailability") { raw } else { report_url() };
    if let Ok(mut url) = url::Url::parse(&base) {
        let params: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| k != "view")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .chain(std::iter::once(("view".to_string(), "week".to_string())))
            .collect();
        let query = params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        url.set_query(Some(&query));
        let _ = goto(page, url.as_str()).await;
        snap(page, "04b_after_weeks_url_fallback").await;
    }
    true
}

// --------------------------------------------------------------------- export

async fn export_csv(page: &Page, browser: &Browser, out_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(out_dir)?;
    let abs = std::path::absolute(out_dir)?;
    browser
        .execute(
            SetDownloadBehaviorParams::builder()
                .behavior(SetDownloadBehaviorBehavior::Allow)
                .download_path(abs.to_string_lossy().to_string())
                .build()
                .map_err(|e| anyhow!(e))?,
        )
        .await?;

    // Actions container
    let container = "#action-buttons.button-menu.action";
    let c = q(container);
    if !eval_bool(page, &format!("!!__cm.find({c}, null, false)")).await {
        bail!("Actions container (#action-buttons) not found or not visible");
    }
    let ul_open = format!("(() => {{ const c = __cm.find({c}, null, false); const ul = c && c.querySelector('ul'); return !!ul && __cm.vis(ul); }})()");
    if !eval_bool(page, &ul_open).await {
        let icon = format!("__cm.mark((() => {{ const c = __cm.find({c}, null, false); if (!c) return null; const i = [...c.querySelectorAll('.menu-dropdown-button')].find(__cm.ok); return i || c; }})())");
        if let Ok(Some(w)) = eval_str(page, &icon).await {
            let _ = click_marked(page, &w).await;
        }
        sleep_ms(300).await;
    }
    if !eval_bool(page, &ul_open).await {
        bail!("Could not open the Actions dropdown");
    }

    let before: std::collections::HashSet<PathBuf> = list_files(&abs);
    let item = format!(
        "__cm.mark((() => {{ const c = __cm.find({c}, null, false); if (!c) return null; const a = c.querySelector('#exportToCSVBtn'); if (a && __cm.ok(a)) return a; return [...c.querySelectorAll('li')].find(l => __cm.ok(l) && /export to csv/i.test(l.innerText || l.textContent || '')) || null; }})())"
    );
    let Some(w) = eval_str(page, &item).await.ok().flatten() else {
        bail!("'Export to csv' item not found inside #action-buttons");
    };
    println!("[app] Clicking 'Export to csv'…");
    click_marked(page, &w).await?;

    // Wait for the finished file (Chrome writes .crdownload first).
    let start = Instant::now();
    let mut saved: Option<PathBuf> = None;
    while start.elapsed() < Duration::from_secs(90) {
        sleep_ms(500).await;
        let now = list_files(&abs);
        let partial = now.iter().any(|p| p.extension().and_then(|e| e.to_str()) == Some("crdownload"));
        if partial {
            continue;
        }
        if let Some(p) = now
            .iter()
            .filter(|p| !before.contains(*p))
            .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("csv") | Some("CSV")))
            .max_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        {
            if fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false) {
                saved = Some(p.clone());
                break;
            }
        }
    }
    let downloaded = saved.context("The export did not finish downloading")?;

    // Name it like the old app did: "<stem>_<today>.csv"
    let stem = downloaded.file_stem().and_then(|s| s.to_str()).unwrap_or("Report").to_string();
    let stem = strip_copy_suffix(&stem);
    let date = chrono::Local::now().format("%Y-%m-%d");
    let target = abs.join(format!("{stem}_{date}.csv"));
    if target != downloaded {
        let _ = fs::remove_file(&target);
        fs::rename(&downloaded, &target).or_else(|_| fs::copy(&downloaded, &target).map(|_| ()).and_then(|_| fs::remove_file(&downloaded)))?;
    }
    println!("[app] CSV downloaded to: {}", target.display());
    Ok(target)
}

fn strip_copy_suffix(stem: &str) -> String {
    // "Report (1)" -> "Report"
    if let Some(i) = stem.rfind(" (") {
        if stem.ends_with(')') && stem[i + 2..stem.len() - 1].chars().all(|c| c.is_ascii_digit()) {
            return stem[..i].to_string();
        }
    }
    stem.to_string()
}

fn list_files(dir: &Path) -> std::collections::HashSet<PathBuf> {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default()
}

// ------------------------------------------------------------------ main flow

async fn open_and_login(page: &Page, username: &str, password: &str) -> Result<()> {
    goto(page, &report_url()).await?;
    snap(page, "00_arrived").await;
    if try_login(page, username, password).await {
        wait_idle(page, 30_000).await;
        snap(page, "01_after_login_click").await;
        // Re-navigate if we drifted off the report.
        let url = page.url().await.ok().flatten().unwrap_or_default();
        if !url.contains("PR_ResourceAvailability") {
            goto(page, &report_url()).await?;
        }
    }
    Ok(())
}

/// Downloads this week's CSV into `out_dir` and returns its path.
pub async fn download_csv(headless: bool, out_dir: PathBuf) -> Result<PathBuf> {
    let username = env::var("CMAP_USER").context("Missing CMAP_USER")?;
    let password = env::var("CMAP_PASS").context("Missing CMAP_PASS")?;
    fs::create_dir_all(profile_dir())?;
    fs::create_dir_all(debug_dir())?;

    let l = launch(Some(&profile_dir()), headless).await?;
    let result = async {
        let page = l.browser.new_page("about:blank").await?;
        println!("[app] navigating to report url…");
        open_and_login(&page, &username, &password).await?;

        println!("[app] looking for 'Run Report'…");
        let run = match click_button(&page, "Run Report").await {
            Some(r) => Some(r),
            None => click_button(&page, "Run").await,
        };
        match run {
            Some(how) => {
                println!("[app] clicking 'Run Report' via {how}");
                wait_idle(&page, 60_000).await;
                snap(&page, "03_after_run_click").await;
            }
            None => println!("[app] 'Run Report' not found — check debug snapshots."),
        }

        go_to_weeks_view(&page).await;

        println!("[app] looking for 'Expand All'…");
        sleep_ms(1200).await;
        match click_button(&page, "Expand All").await {
            Some(how) => {
                println!("[app] clicking 'Expand All' via {how}");
                wait_idle(&page, 10_000).await;
                snap(&page, "05_after_expand_click").await;
            }
            None => println!("[app] 'Expand All' not found — may be in a menu."),
        }

        let r = export_csv(&page, &l.browser, &out_dir).await;
        if r.is_err() {
            snap(&page, "06_export_failed").await;
        }
        println!("[app] Finished all steps. Closing browser…");
        r
    }
    .await;
    l.close().await;
    result.context("cmap export did not produce a CSV (see .cmap_browser\\debug for snapshots)")
}

// ----------------------------------------------------------- project lookup

/// Looks up the full "Project No - Title" for each truncated name by typing the
/// project number into the Projects tab filter. Returns truncated name -> full name.
pub async fn lookup_project_names(headless: bool, truncated: &[String]) -> Result<HashMap<String, String>> {
    let mut found = HashMap::new();
    if truncated.is_empty() {
        return Ok(found);
    }
    let username = env::var("CMAP_USER").context("Missing CMAP_USER")?;
    let password = env::var("CMAP_PASS").context("Missing CMAP_PASS")?;
    fs::create_dir_all(profile_dir())?;
    fs::create_dir_all(debug_dir())?;

    let l = launch(Some(&profile_dir()), headless).await?;
    let result = lookup_steps(&l.browser, &username, &password, truncated, &mut found).await;
    l.close().await;
    result?;
    Ok(found)
}

async fn lookup_steps(
    browser: &Browser,
    username: &str,
    password: &str,
    truncated: &[String],
    found: &mut HashMap<String, String>,
) -> Result<()> {
    let page = browser.new_page("about:blank").await?;
    let rurl = report_url();
    let base = rurl.split("/Reports").next().unwrap_or(&rurl).to_string();
    goto(&page, &base).await?;
    if try_login(&page, username, password).await {
        wait_idle(&page, 30_000).await;
    }

    // Top navigation -> Projects
    let nav = "__cm.mark(__cm.find('a', 'Projects', true))";
    let Some(w) = eval_str(&page, nav).await.ok().flatten() else {
        snap(&page, "10_projects_nav_missing").await;
        bail!("could not find the 'Projects' tab in cmap (see debug snapshots)");
    };
    click_marked(&page, &w).await?;
    wait_idle(&page, 30_000).await;
    sleep_ms(1500).await;
    snap(&page, "10_projects_page").await;

    for name in truncated {
        let code = name.split(" - ").next().unwrap_or(name).trim().to_string();
        let prefix = name
            .trim_end_matches(|c| c == '.' || c == '…')
            .trim_end()
            .to_string();
        if code.is_empty() {
            continue;
        }
        // The text box under the "Project No - Title" column header.
        let filter_js = r#"(() => {
            const hdr = __cm.leaf('Project No - Title', true);
            if (!hdr) return null;
            const hb = hdr.getBoundingClientRect();
            let best = null, bd = 1e9;
            for (const d of __cm.docs()) for (const i of d.querySelectorAll('input[type="text"],input:not([type])')) {
              if (!__cm.ok(i)) continue;
              const b = i.getBoundingClientRect();
              if (b.top <= hb.top) continue;
              const dist = Math.abs(b.left - hb.left);
              if (dist < bd) { bd = dist; best = i; }
            }
            return best && bd < 80 ? __cm.mark(best) : null;
        })()"#;
        let Some(w) = eval_str(&page, filter_js).await.ok().flatten() else {
            snap(&page, "11_filter_box_missing").await;
            println!("[lookup] could not find the 'Project No - Title' filter box");
            break;
        };
        if !type_into(&page, &w, &code).await {
            println!("[lookup] could not type into the filter box");
            break;
        }
        press_on_marked(&page, "Enter").await;

        let mut matches: Vec<String> = Vec::new();
        for _ in 0..12 {
            sleep_ms(500).await;
            let js = format!(
                "JSON.stringify([...new Set([...__cm.docs()].flatMap(d => [...d.querySelectorAll('a')]).map(a => __cm.txt(a)).filter(t => t.startsWith({}) && t.length > {}))])",
                q(&prefix),
                prefix.chars().count()
            );
            if let Ok(Some(s)) = eval_str(&page, &js).await {
                matches = serde_json::from_str(&s).unwrap_or_default();
            }
            if !matches.is_empty() {
                break;
            }
        }
        matches.sort();
        matches.dedup();
        match matches.len() {
            1 => {
                println!("[lookup] {name}  ->  {}", matches[0]);
                found.insert(name.clone(), matches.remove(0));
            }
            0 => {
                snap(&page, &format!("11_lookup_{}", code.replace(['/', '\\', ':'], "_"))).await;
                println!("[lookup] {name}: no project starting with that name was listed");
            }
            n => println!("[lookup] {name}: {n} different projects match, not adding any: {matches:?}"),
        }
        // clear the filter for the next lookup
        if let Ok(Some(w)) = eval_str(&page, filter_js).await {
            if type_into(&page, &w, "").await {
                press_on_marked(&page, "Backspace").await;
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------- PNG export

pub async fn render_pngs(jobs: &[(PathBuf, PathBuf)]) -> Result<()> {
    if jobs.is_empty() {
        return Ok(());
    }
    let l = launch(None, true).await?;
    let result = async {
        let page = l.browser.new_page("about:blank").await?;
        page.execute(SetDeviceMetricsOverrideParams::new(
            EXPORT_WIDTH as i64,
            EXPORT_HEIGHT as i64,
            2.0,
            false,
        ))
        .await?;
        page.execute(SetDefaultBackgroundColorOverrideParams {
            color: Some(Rgba { r: 0, g: 0, b: 0, a: Some(0.0) }),
        })
        .await?;
        for (html, png) in jobs {
            let abs = std::path::absolute(html)?;
            let url = url::Url::from_file_path(&abs).map_err(|_| anyhow!("bad path {}", abs.display()))?;
            page.goto(url.as_str()).await?;
            wait_idle(&page, 15_000).await;
            let el = page.find_element(".page").await.context("dashboard .page element not found")?;
            let bytes = el.screenshot(CaptureScreenshotFormat::Png).await?;
            match write_png(png, &bytes) {
                Ok(path) => println!("[app] wrote {}", path.display()),
                Err(e) => println!("[warn] could not save {}: {e}", png.display()),
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    l.close().await;
    result
}

/// Windows refuses to overwrite a file that is open in a viewer or the Explorer preview pane
/// (os error 1224 / 32). Try the normal name first, then fall back to a "_new" copy.
fn write_png(png: &PathBuf, bytes: &[u8]) -> Result<PathBuf> {
    if std::fs::write(png, bytes).is_ok() {
        return Ok(png.clone());
    }
    let _ = std::fs::remove_file(png);
    if std::fs::write(png, bytes).is_ok() {
        return Ok(png.clone());
    }
    let stem = png.file_stem().and_then(|s| s.to_str()).unwrap_or("dashboard");
    let alt = png.with_file_name(format!("{stem}_new.png"));
    std::fs::write(&alt, bytes)
        .with_context(|| format!("{} is open in another program; close it and run again", png.display()))?;
    println!(
        "[warn] {} is open in another program, saved as {} instead",
        png.display(),
        alt.display()
    );
    Ok(alt)
}
