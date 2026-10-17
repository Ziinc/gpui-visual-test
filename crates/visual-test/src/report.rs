//! `index.html`: every shot in the directory, failing and pending first.

use crate::{Status, Store, store::write_atomic};
use anyhow::Result;
use std::path::PathBuf;

/// Rewrites `index.html` in the store and returns its path.
pub fn write_index(store: &Store) -> Result<PathBuf> {
    let mut manifests = store.list()?;
    manifests.sort_by_key(|m| (rank(m.status()), m.name.clone()));
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>Visual tests</title><style>\
         body{font:14px system-ui;margin:24px;background:#fafafa;color:#222}\
         section{background:#fff;border:1px solid #ddd;border-radius:8px;padding:16px;margin:0 0 16px}\
         img{max-width:100%;border:1px solid #eee}li{margin:4px 0}\
         .fail{color:#b00020}.unsure{color:#8a6d00}.pass{color:#1b7a1b}small{color:#666}\
         @media(prefers-color-scheme:dark){body{background:#111;color:#ddd}section{background:#1b1b1b;border-color:#333}img{border-color:#333}}\
         </style><h1>Visual tests</h1>",
    );
    for m in &manifests {
        let image = format!("{}.png", m.name);
        html.push_str(&format!(
            "<section><h2 class={status}>{name} <small>{status}</small></h2><a href=\"{image}\"><img src=\"{image}\" alt=\"{name}\"></a>",
            status = m.status(),
            name = escape(&m.name),
            image = href(&image),
        ));
        if let Some(diff) = &m.diff {
            html.push_str(&format!(
                "<p class={status}><b>pixel diff {status}</b> {}</p>",
                escape(&diff.reason),
                status = diff.status,
            ));
            if let Some(image) = &diff.image {
                let dir = std::path::Path::new(&m.name)
                    .parent()
                    .map(|p| format!("{}/", p.display()))
                    .unwrap_or_default();
                let image = href(&format!("{dir}{image}"));
                html.push_str(&format!(
                    "<a href=\"{image}\"><img src=\"{image}\" alt=\"pixel diff\"></a>"
                ));
            }
        }
        html.push_str("<ol start=0>");
        for entry in &m.checks {
            let status = entry.status();
            html.push_str(&format!(
                "<li class={status}><b>{status}</b> {}",
                escape(&entry.expectation.text)
            ));
            if let Some(v) = &entry.verdict {
                html.push_str(&format!(
                    "<br><small>{} ({})</small>",
                    escape(&v.reason),
                    escape(&v.judge)
                ));
            }
            html.push_str("</li>");
        }
        html.push_str("</ol></section>");
    }
    let path = store.dir().join("index.html");
    write_atomic(&path, html.as_bytes())?;
    Ok(path)
}

fn rank(status: Status) -> u8 {
    match status {
        Status::Fail => 0,
        Status::Unsure => 1,
        Status::Pass => 2,
    }
}

/// A relative link to a file: each byte outside the URL-safe set is
/// percent-encoded, so names with `#`, `?`, `%` or spaces still resolve.
fn href(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    #[test]
    fn hrefs_encode_url_syntax() {
        assert_eq!(super::href("a b/c#1?.png"), "a%20b/c%231%3F.png");
        assert_eq!(super::href("ui/é.png"), "ui/%C3%A9.png");
    }
}
