//! Finds an installed Chrome or Edge to drive.
use std::path::PathBuf;

pub fn system_browser() -> Option<PathBuf> {
    // Manual override (also handy for testing): CMAP_BROWSER=C:\path\to\chrome.exe
    if let Some(p) = std::env::var_os("CMAP_BROWSER") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(v) = std::env::var_os(var) {
            roots.push(PathBuf::from(v));
        }
    }
    let rel = [
        r"Google\Chrome\Application\chrome.exe",
        r"Microsoft\Edge\Application\msedge.exe",
    ];
    for r in &roots {
        for p in rel {
            let c = r.join(p);
            if c.exists() {
                return Some(c);
            }
        }
    }
    // Non-Windows fallbacks (development only).
    for p in ["/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"] {
        if std::path::Path::new(p).exists() {
            return Some(PathBuf::from(p));
        }
    }
    None
}
