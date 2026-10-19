//! cmap login: taken from CMAP_USER / CMAP_PASS (.env) if set, otherwise asked for once
//! and remembered in Windows Credential Manager (never written to a file).
use anyhow::{Context, Result};
use std::io::Write;

const SERVICE: &str = "cmap-dashboard";
const ENTRY: &str = "cmap-login";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ENTRY).context("could not open the Windows Credential Manager")
}

/// Makes sure CMAP_USER and CMAP_PASS are set in this process.
pub fn ensure_credentials() -> Result<()> {
    let have = |k: &str| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false);
    if have("CMAP_USER") && have("CMAP_PASS") {
        return Ok(());
    }
    if let Ok(secret) = entry().and_then(|e| e.get_password().map_err(Into::into)) {
        if let Some((u, p)) = secret.split_once('\n') {
            std::env::set_var("CMAP_USER", u);
            std::env::set_var("CMAP_PASS", p);
            return Ok(());
        }
    }

    println!("\nFirst-time setup: enter your own cmap login.");
    println!("It is stored in Windows Credential Manager on this computer only.\n");
    print!("cmap username (email): ");
    std::io::stdout().flush().ok();
    let mut user = String::new();
    std::io::stdin().read_line(&mut user)?;
    let user = user.trim().to_string();
    let pass = rpassword::prompt_password("cmap password (hidden): ")?;
    if user.is_empty() || pass.is_empty() {
        anyhow::bail!("a username and password are both needed");
    }
    match entry().and_then(|e| e.set_password(&format!("{user}\n{pass}")).map_err(Into::into)) {
        Ok(_) => println!("[app] Login saved. Use --forget-login to clear it.\n"),
        Err(e) => println!("[warn] Could not save the login ({e:#}); you will be asked again next time.\n"),
    }
    std::env::set_var("CMAP_USER", user);
    std::env::set_var("CMAP_PASS", pass);
    Ok(())
}

/// Removes the saved login.
pub fn forget() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
    println!("[app] Saved cmap login cleared.");
}
