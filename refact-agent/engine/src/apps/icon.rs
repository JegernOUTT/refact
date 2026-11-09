//! Untrusted bundle SVG is reduced to bounded geometry, never forwarded as markup.
use std::collections::BTreeMap;
use refact_apps_core::PublishedBundle;
use serde::Serialize;

const MAX_BYTES: usize = 16 * 1024;
const MAX_NODES: usize = 64;
const MAX_DEPTH: usize = 8;
const MAX_VALUE: f64 = 4096.0;

#[derive(Debug, Serialize)]
pub struct Artwork {
    view_box: [f64; 4],
    nodes: Vec<Geometry>,
}
#[derive(Debug, Serialize)]
struct Geometry {
    tag: String,
    attrs: BTreeMap<String, String>,
    children: Vec<Geometry>,
}

pub fn project(bundle: &PublishedBundle) -> Option<Artwork> {
    let path = bundle.manifest.icon.as_deref()?;
    if !path.ends_with(".svg") || refact_apps_core::validate_resource_path(path).is_err() {
        return None;
    }
    parse(bundle.files.get(path)?).ok()
}

fn numbers(value: &str) -> Result<Vec<f64>, ()> {
    // SVG permits adjacent signed numbers. Match the whole input, not just its
    // numeric substrings (otherwise url()/NaN/units could slip through).
    static NUMBER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = NUMBER.get_or_init(|| {
        regex::Regex::new(r"[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?").unwrap()
    });
    let mut end = 0;
    let mut output = Vec::new();
    for token in regex.find_iter(value) {
        if !value[end..token.start()]
            .bytes()
            .all(|b| b.is_ascii_whitespace() || b == b',')
        {
            return Err(());
        }
        let number: f64 = token.as_str().parse().map_err(|_| ())?;
        if !number.is_finite() || number.abs() > MAX_VALUE || output.len() >= 1024 {
            return Err(());
        }
        output.push(number);
        end = token.end();
    }
    if output.is_empty()
        || !value[end..]
            .bytes()
            .all(|b| b.is_ascii_whitespace() || b == b',')
    {
        return Err(());
    }
    Ok(output)
}

fn paint(value: &str) -> bool {
    matches!(value, "none" | "currentColor")
        || value.strip_prefix('#').is_some_and(|hex| {
            matches!(hex.len(), 3 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
        })
}
fn attribute(tag: &str, name: &str, value: &str) -> bool {
    if value.len() > 4096 {
        return false;
    }
    match name {
        "fill" | "stroke" => paint(value),
        "stroke-linecap" => matches!(value, "butt" | "round" | "square"),
        "stroke-linejoin" => matches!(value, "miter" | "round" | "bevel"),
        "fill-rule" | "clip-rule" => matches!(value, "nonzero" | "evenodd"),
        "d" if tag == "path" => {
            if !value.starts_with(['M', 'm']) {
                return false;
            }
            let numeric = value
                .chars()
                .map(|c| {
                    if "MmZzLlHhVvCcSsQqTtAa".contains(c) {
                        ' '
                    } else {
                        c
                    }
                })
                .collect::<String>();
            numbers(&numeric).is_ok()
        }
        "points" if matches!(tag, "polyline" | "polygon") => {
            numbers(value).is_ok_and(|v| v.len() >= 4 && v.len() % 2 == 0)
        }
        _ => {
            let allowed = match tag {
                "svg" | "g" | "path" | "polyline" | "polygon" => false,
                "line" => matches!(name, "x1" | "y1" | "x2" | "y2"),
                "rect" => matches!(name, "x" | "y" | "width" | "height" | "rx" | "ry"),
                "circle" => matches!(name, "cx" | "cy" | "r"),
                "ellipse" => matches!(name, "cx" | "cy" | "rx" | "ry"),
                _ => false,
            } || matches!(
                name,
                "stroke-width"
                    | "stroke-miterlimit"
                    | "opacity"
                    | "fill-opacity"
                    | "stroke-opacity"
            );
            allowed
                && numbers(value).is_ok_and(|v| {
                    v.len() == 1
                        && match name {
                            "opacity" | "fill-opacity" | "stroke-opacity" => {
                                (0.0..=1.0).contains(&v[0])
                            }
                            "width" | "height" | "r" | "rx" | "ry" | "stroke-width"
                            | "stroke-miterlimit" => v[0] >= 0.0,
                            _ => true,
                        }
                })
        }
    }
}

fn geometry(
    node: roxmltree::Node<'_, '_>,
    depth: usize,
    count: &mut usize,
) -> Result<Geometry, ()> {
    *count += 1;
    let tag = node.tag_name().name();
    if depth > MAX_DEPTH
        || *count > MAX_NODES
        || !matches!(
            node.tag_name().namespace(),
            None | Some("http://www.w3.org/2000/svg")
        )
        || !matches!(
            tag,
            "svg" | "g" | "path" | "line" | "polyline" | "polygon" | "rect" | "circle" | "ellipse"
        )
        || (tag == "svg" && depth != 0)
    {
        return Err(());
    }
    let mut attrs = BTreeMap::new();
    for attr in node.attributes() {
        if depth == 0 && attr.name() == "viewBox" && attr.namespace().is_none() {
            continue;
        }
        if attr.namespace().is_some() || !attribute(tag, attr.name(), attr.value()) {
            return Err(());
        }
        attrs.insert(attr.name().to_owned(), attr.value().to_owned());
    }
    let mut children = Vec::new();
    for child in node.children() {
        if child.is_element() {
            if !matches!(tag, "svg" | "g") {
                return Err(());
            }
            children.push(geometry(child, depth + 1, count)?);
        } else if !child.is_text() || !child.text().unwrap_or("").trim().is_empty() {
            return Err(());
        }
    }
    Ok(Geometry {
        tag: tag.to_owned(),
        attrs,
        children,
    })
}

fn parse(bytes: &[u8]) -> Result<Artwork, ()> {
    if bytes.len() > MAX_BYTES {
        return Err(());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    // Refuse DTD, entities, comments, CDATA and processing instructions before
    // XML parsing. There is no resolver and no XML bytes ever reach the browser.
    if text.contains("<!") || text.contains("<?") || text.contains('&') {
        return Err(());
    }
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 256,
        },
    )
    .map_err(|_| ())?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" {
        return Err(());
    }
    let view = numbers(root.attribute("viewBox").ok_or(())?)?;
    let view_box: [f64; 4] = view.try_into().map_err(|_| ())?;
    if view_box[2] <= 0.0 || view_box[3] <= 0.0 {
        return Err(());
    }
    let mut node = geometry(root, 0, &mut 0)?;
    // Root presentation attributes remain inert by rendering them on a group.
    node.tag = "g".into();
    Ok(Artwork {
        view_box,
        nodes: vec![node],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_svg_security_boundary() {
        let valid = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor"><g><path d="M2 3L12 20 22 3Z"/><circle cx="12" cy="9" r="2" fill="#aabbcc"/></g></svg>"##;
        let artwork = serde_json::to_value(parse(valid).unwrap()).unwrap();
        assert_eq!(
            artwork["nodes"][0]["children"][0]["children"][0]["tag"],
            "path"
        );
        for payload in [
            "<script/>",
            "<foreignObject/>",
            "<image href='https://evil.test/x'/>",
            "<use href='#x'/>",
            "<a href='https://evil.test'/>",
            "<path onclick='alert(1)'/>",
            "<path style='fill:red'/>",
            "<path fill='url(https://evil.test)'/>",
            "<path d='M0 0L1e99 0'/>",
        ] {
            assert!(
                parse(format!("<svg viewBox='0 0 24 24'>{payload}</svg>").as_bytes()).is_err(),
                "{payload}"
            );
        }
        assert!(
            parse(b"<!DOCTYPE svg [<!ENTITY x 'hi'>]><svg viewBox='0 0 24 24'>&x;</svg>").is_err()
        );
        assert!(parse(
            format!("<svg viewBox='0 0 24 24'>{}</svg>", "<path/>".repeat(65)).as_bytes()
        )
        .is_err());
        assert!(parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        assert!(parse(b"<svg viewBox='0 0 0 24'/>").is_err());
    }
    #[tokio::test]
    async fn catalog_artwork_is_from_the_record_bundle_and_invalid_artwork_is_optional() {
        let root = tempfile::tempdir().unwrap();
        let store = refact_apps_core::AppStore::open(root.path()).await.unwrap();
        let app = store.create("icon-test", "Icon test").await.unwrap();
        let app = store
            .write_file(
                "icon-test",
                "assets/icon.svg",
                b"<svg viewBox='0 0 24 24'><circle cx='12' cy='12' r='8'/></svg>".to_vec(),
                &app.revision,
            )
            .await
            .unwrap();
        let mut manifest = app.manifest.clone();
        manifest.icon = Some("assets/icon.svg".into());
        let app = store
            .write_file(
                "icon-test",
                "app.json",
                serde_json::to_vec(&manifest).unwrap(),
                &app.revision,
            )
            .await
            .unwrap();
        store.publish("icon-test", &app.revision).await.unwrap();
        let (listing, icons) = store
            .list_projected(|bundle| (bundle.revision.clone(), project(bundle)))
            .await
            .unwrap();
        assert_eq!(icons["icon-test"].0, listing.apps[0].revision);
        assert!(icons["icon-test"].1.is_some());
        let draft = store
            .write_file(
                "icon-test",
                "assets/icon.svg",
                b"<svg viewBox='0 0 24 24'><script/></svg>".to_vec(),
                &app.revision,
            )
            .await
            .unwrap();
        let (listing, icons) = store.list_projected(project).await.unwrap();
        assert_eq!(listing.apps[0].revision, draft.revision);
        assert_eq!(
            listing.apps[0].active_revision.as_deref(),
            Some(app.revision.as_str())
        );
        assert!(icons["icon-test"].is_none());
        assert_eq!(listing.apps.len(), 1);
    }
}
