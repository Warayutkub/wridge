//! Google Fonts that reports ask for by name (theme.font, a block's style.font), kept on this computer:
//! the first time a family is used wridge fetches its CSS and font files and stores them under
//! `<app dir>/font-cache/`; after that — also offline, and in the headless browser that exports PNG/PDF —
//! the page gets them from here. IBM Plex Sans Thai and IBM Plex Mono ship with the app (web/fonts/).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};

/// Google sends WOFF2 only to browsers it recognises; any current desktop Chrome will do.
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

pub fn cache_dir(app_dir: &Path) -> PathBuf {
    app_dir.join("font-cache")
}

/// "Kanit", "Noto Serif Thai" … letters, digits and spaces only.
fn check_family(family: &str) -> anyhow::Result<String> {
    let f = family.trim();
    if f.is_empty() || f.len() > 60 || !f.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') {
        bail!("font family: letters, digits and spaces, e.g. \"Kanit\"");
    }
    Ok(f.to_string())
}

fn css_file(dir: &Path, family: &str) -> PathBuf {
    dir.join(format!("{}.css", family.to_lowercase().replace(' ', "-")))
}

/// The family's @font-face CSS, its url()s pointing at /api/fonts/file. From the cache, else fetched and cached.
pub async fn css(app_dir: &Path, family: &str) -> anyhow::Result<String> {
    let family = check_family(family)?;
    let dir = cache_dir(app_dir);
    let cached = css_file(&dir, &family);
    if let Ok(text) = std::fs::read_to_string(&cached) {
        return Ok(text);
    }
    let client = reqwest::Client::builder().user_agent(BROWSER_UA).timeout(std::time::Duration::from_secs(15)).build()?;
    let name = family.replace(' ', "+");
    // the weights reports use; a family that lacks some of them answers 400, so ask again for its default
    let mut text = None;
    for url in [format!("https://fonts.googleapis.com/css2?family={name}:wght@300;400;500;600;700&display=block"), format!("https://fonts.googleapis.com/css2?family={name}&display=block")] {
        let r = client.get(&url).send().await.map_err(|e| anyhow!("couldn't reach Google Fonts: {e}"))?;
        if r.status().is_success() {
            text = Some(r.text().await?);
            break;
        }
    }
    let text = text.ok_or_else(|| anyhow!("Google Fonts has no family \"{family}\""))?;
    std::fs::create_dir_all(&dir)?;
    // fetch each file once, rewrite its url()
    // (matches collected first: nothing from the regex may live across an await)
    let found: Vec<(std::ops::Range<usize>, String)> = {
        let re = regress::Regex::new(r"url\((https://fonts\.gstatic\.com/[^)]+)\)").unwrap();
        re.find_iter(&text).map(|m| (m.range(), text[m.group(1).unwrap()].to_string())).collect()
    };
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, url) in found {
        let url = url.as_str();
        let file = url.trim_start_matches("https://fonts.gstatic.com/").replace(['/', '\\'], "-");
        let file: String = file.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect();
        let path = dir.join(&file);
        if !path.exists() {
            let bytes = client.get(url).send().await?.error_for_status()?.bytes().await?;
            std::fs::write(&path, &bytes)?;
        }
        out.push_str(&text[last..range.start]);
        out.push_str(&format!("url(/api/fonts/file?name={file})"));
        last = range.end;
    }
    out.push_str(&text[last..]);
    std::fs::write(&cached, &out)?;
    Ok(out)
}

/// A cached font file, by the name the CSS gave it.
pub fn file(app_dir: &Path, name: &str) -> anyhow::Result<(Vec<u8>, &'static str)> {
    if name.is_empty() || name.contains("..") || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        bail!("no such font file");
    }
    let kind = match name.rsplit('.').next() {
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        _ => bail!("no such font file"),
    };
    let bytes = std::fs::read(cache_dir(app_dir).join(name)).map_err(|_| anyhow!("no such font file"))?;
    Ok((bytes, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert!(check_family("Noto Serif Thai").is_ok());
        assert!(check_family("../x").is_err());
        assert!(check_family("").is_err());
        let d = std::env::temp_dir();
        assert!(file(&d, "../secret.woff2").is_err());
        assert!(file(&d, "x.exe").is_err());
    }
}
