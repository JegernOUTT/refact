use refact_apps_core::PublishedBundle;
use base64::Engine;
fn resolve(base: &str, target: &str) -> Option<String> {
    if target.contains(':') || target.starts_with('/') || target.starts_with('#') {
        return None;
    }
    let mut parts = base
        .rsplit_once('/')
        .map(|(p, _)| p.split('/').collect::<Vec<_>>())
        .unwrap_or_default();
    for p in target.split('/') {
        match p {
            "." => {}
            ".." => {
                parts.pop()?;
            }
            "" => return None,
            _ => parts.push(p),
        }
    }
    let path = parts.join("/");
    refact_apps_core::validate_resource_path(&path).ok()?;
    Some(path)
}
fn data(path: &str, bytes: &[u8]) -> String {
    let mime = match path.rsplit('.').next().unwrap_or("") {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    };
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}
fn css(bundle: &PublishedBundle, path: &str, text: &str) -> String {
    let re = regex::Regex::new(r#"url\(\s*[\"']?([^\s\)\"']+)[\"']?\s*\)"#).unwrap();
    re.replace_all(text, |c: &regex::Captures| {
        resolve(path, &c[1])
            .and_then(|p| {
                bundle
                    .files
                    .get(&p)
                    .map(|b| format!("url(\"{}\")", data(&p, b)))
            })
            .unwrap_or_else(|| c[0].into())
    })
    .into_owned()
}
pub fn inline(bundle: &PublishedBundle) -> Result<String, String> {
    let path = &bundle.manifest.entrypoint;
    let html = std::str::from_utf8(bundle.files.get(path).ok_or("entrypoint missing")?)
        .map_err(|_| "entrypoint is not UTF8")?;
    let scripts = regex::Regex::new(
        r#"(?is)<script\b[^>]*\bsrc\s*=\s*[\"']([^\"']+)[\"'][^>]*>\s*</script\s*>"#,
    )
    .unwrap();
    let html = scripts.replace_all(html, |c: &regex::Captures| {
        resolve(path, &c[1])
            .and_then(|p| bundle.files.get(&p))
            .and_then(|b| std::str::from_utf8(b).ok())
            .map(|s| {
                format!(
                    "<script>{}</script>",
                    s.replace("</script", "<\\/script")
                        .replace("</SCRIPT", "<\\/SCRIPT")
                )
            })
            .unwrap_or_else(|| c[0].into())
    });
    let links =
        regex::Regex::new(r#"(?is)<link\b[^>]*\bhref\s*=\s*[\"']([^\"']+\.css)[\"'][^>]*>"#)
            .unwrap();
    let html = links.replace_all(&html, |c: &regex::Captures| {
        resolve(path, &c[1])
            .and_then(|p| {
                bundle
                    .files
                    .get(&p)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .map(|s| {
                        format!(
                            "<style>{}</style>",
                            css(bundle, &p, s).replace("</style", "<\\/style")
                        )
                    })
            })
            .unwrap_or_else(|| c[0].into())
    });
    let assets = regex::Regex::new(r#"(?i)\b(src|poster)\s*=\s*[\"']([^\"']+)[\"']"#).unwrap();
    let html = assets
        .replace_all(&html, |c: &regex::Captures| {
            resolve(path, &c[2])
                .and_then(|p| {
                    bundle
                        .files
                        .get(&p)
                        .map(|b| format!("{}=\"{}\"", &c[1], data(&p, b)))
                })
                .unwrap_or_else(|| c[0].into())
        })
        .into_owned();
    if html.len() > 5_000_000 {
        return Err("inlined document exceeds host limit".into());
    }
    Ok(html)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_resolution_cannot_escape_bundle() {
        assert_eq!(
            resolve("ui/index.html", "../assets/a.png").unwrap(),
            "assets/a.png"
        );
        assert!(resolve("ui/index.html", "../../secret").is_none());
        assert!(resolve("ui/index.html", "https://evil").is_none());
    }
}
