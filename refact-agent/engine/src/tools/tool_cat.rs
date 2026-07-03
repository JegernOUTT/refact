use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use serde_json::Value;
use itertools::Itertools;

use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use resvg::{tiny_skia, usvg};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::files_correction::{
    canonical_path, correct_to_nearest_dir_path, get_project_dirs,
    preprocess_path_for_normalization,
};
use crate::files_in_workspace::{
    check_file_privacy_for_send, get_file_text_from_memory_or_disk, ls_files,
};
use crate::scratchpads::multimodality::MultimodalElement;
use crate::knowledge_index::format_related_memories_section;
use crate::tools::scope_utils::{
    format_scope_notices, list_scoped_files_under_dir, resolve_existing_path_with_execution_scope,
};

use std::io::Cursor;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};

pub struct ToolCat {
    pub config_path: String,
}

const CAT_MAX_IMAGES_CNT: usize = 1;
const CAT_MAX_LINES: usize = 2000;

type CatLineRange = (usize, usize);

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatPathRequest {
    path: String,
    line_range: Option<CatLineRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatResolvedSource {
    ExplicitFile,
    DirectoryExpansion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatResolvedPath {
    path: String,
    line_range: Option<CatLineRange>,
    source: CatResolvedSource,
}

fn parse_cat_args(
    args: &HashMap<String, Value>,
) -> Result<(Vec<CatPathRequest>, Vec<String>), String> {
    fn try_parse_line_range(s: &str) -> Result<Option<(usize, usize)>, String> {
        let s = s.trim();

        // Try parsing as a single number (like "10")
        if let Ok(n) = s.parse::<usize>() {
            return Ok(Some((n, n)));
        }

        // Try parsing as a range (like "10-20")
        if s.contains('-') {
            let parts = s.split('-').collect::<Vec<_>>();
            if parts.len() == 2 {
                if let Ok(start) = parts[0].trim().parse::<usize>() {
                    if let Ok(end) = parts[1].trim().parse::<usize>() {
                        if start > end {
                            return Err(format!(
                                "Start line ({}) cannot be greater than end line ({})",
                                start, end
                            ));
                        }
                        return Ok(Some((start, end)));
                    }
                }
            }
        }

        Ok(None) // Not a line range - likely a Windows path
    }

    let raw_paths = match args.get("paths") {
        Some(Value::String(s)) => s
            .split(",")
            .map(|x| x.trim().to_string())
            .collect::<Vec<_>>(),
        Some(v) => return Err(format!("argument `paths` is not a string: {:?}", v)),
        None => return Err("Missing argument `paths`".to_string()),
    };

    let mut paths = Vec::new();

    for path_str in raw_paths {
        let (file_path, range) = if let Some(colon_pos) = path_str.rfind(':') {
            match try_parse_line_range(&path_str[colon_pos + 1..])? {
                Some((start, end)) => {
                    (path_str[..colon_pos].trim().to_string(), Some((start, end)))
                }
                None => (path_str, None),
            }
        } else {
            (path_str, None)
        };
        paths.push(CatPathRequest {
            path: file_path,
            line_range: range,
        });
    }

    let symbols = match args.get("symbols") {
        Some(Value::String(s)) => {
            if s == "*" {
                vec![]
            } else {
                s.split(",")
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
            }
        }
        Some(v) => return Err(format!("argument `symbols` is not a string: {:?}", v)),
        None => vec![],
    };

    Ok((paths, symbols))
}

#[async_trait]
impl Tool for ToolCat {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "cat".to_string(),
            display_name: "Cat".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Like cat in console, but better: it can read multiple files and images. Prefer to open full files.".to_string(),
            input_schema: json_schema_from_params(&[("paths", "string", "Comma separated file names or directories: dir1/file1.ext,dir3/dir4.")], &["paths"]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let mut corrections = false;
        let (paths, symbols) = parse_cat_args(args)?;
        let (
            filenames_present,
            symbols_not_found,
            not_found_messages,
            context_enums,
            multimodal,
            scope_notices,
        ) = paths_and_symbols_to_cat_with_path_ranges(ccx.clone(), paths, symbols).await;

        let mut content = format_scope_notices(&scope_notices);
        if !filenames_present.is_empty() {
            content.push_str(&format!(
                "Paths found:\n{}\n\n",
                filenames_present
                    .iter()
                    .unique()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
            if !symbols_not_found.is_empty() {
                content.push_str(&format!(
                    "Symbols not found in the {} files:\n{}\n\n",
                    filenames_present.len(),
                    symbols_not_found.join("\n")
                ));
                corrections = true;
            }
        }
        if !not_found_messages.is_empty() {
            content.push_str(&format!(
                "Problems:\n{}\n\n",
                not_found_messages.join("\n\n")
            ));
            corrections = true;
        }
        if content.is_empty() {
            content = "No files or symbols found matching the request.".to_string();
        }

        let mut results: Vec<ContextEnum> = context_enums
            .into_iter()
            .map(|ctx| {
                if let ContextEnum::ContextFile(mut cf) = ctx {
                    cf.skip_pp = true;
                    ContextEnum::ContextFile(cf)
                } else {
                    ctx
                }
            })
            .collect();

        // Append related memories (short form) based on involved file paths.
        // This is fast: uses in-memory KnowledgeIndex only.
        let related_section = {
            let gcx = ccx.lock().await.app.gcx.clone();
            let idx_arc = { gcx.knowledge_index.clone() };
            let idx_guard = idx_arc.lock().await;
            let mut cards = idx_guard.related_for_files(&filenames_present, 8);
            if cards.is_empty() {
                cards = idx_guard.related_for_related_files(&filenames_present, 8);
            }
            format_related_memories_section(&cards, None)
        };

        let chat_content = if multimodal.is_empty() {
            ChatContent::SimpleText(content)
        } else {
            ChatContent::Multimodal(
                [
                    vec![MultimodalElement {
                        m_type: "text".to_string(),
                        m_content: content,
                    }],
                    multimodal,
                ]
                .concat(),
            )
        };

        // Keep multimodal shape intact: we append a new text block.
        results.push(ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: match chat_content {
                ChatContent::SimpleText(t) => {
                    ChatContent::SimpleText(format!("{}{}", t, related_section))
                }
                ChatContent::Multimodal(mut mm) => {
                    if !related_section.is_empty() {
                        mm.push(MultimodalElement {
                            m_type: "text".to_string(),
                            m_content: related_section,
                        });
                    }
                    ChatContent::Multimodal(mm)
                }
                other => other,
            },
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        }));

        Ok((corrections, results))
    }
}

// todo: we can extract if from pipe, however PathBuf does not implement it
fn get_file_type(path: &PathBuf) -> String {
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if ["png", "svg", "jpeg"].contains(&extension.as_str()) {
        return format!("image/{extension}");
    }
    if ["jpg", "JPG", "JPEG"].contains(&extension.as_str()) {
        return "image/jpeg".to_string();
    }
    return "text".to_string();
}

async fn load_image(path: &String, f_type: &String) -> Result<MultimodalElement, String> {
    let extension = path.split(".").last().unwrap().to_string();
    let mut f_type = f_type.clone();

    let max_dimension = 800;
    let data = match f_type.as_str() {
        "image/png" | "image/jpeg" => {
            let reader =
                ImageReader::open(path).map_err(|_| format!("{} image read failed", path))?;
            let mut image = reader
                .decode()
                .map_err(|_| format!("{} image decode failed", path))?;
            let scale_factor =
                max_dimension as f32 / std::cmp::max(image.width(), image.height()) as f32;
            if scale_factor < 1.0 {
                let (nwidth, nheight) = (
                    scale_factor * image.width() as f32,
                    scale_factor * image.height() as f32,
                );
                image = image.resize(nwidth as u32, nheight as u32, FilterType::Lanczos3);
            }
            let mut data = Vec::new();
            image
                .write_to(&mut Cursor::new(&mut data), ImageFormat::Png)
                .map_err(|_| format!("{} image encode failed", path))?;
            f_type = "image/png".to_string();
            Ok(data)
        }
        "image/svg" => {
            f_type = "image/png".to_string();
            let tree = {
                let mut opt = usvg::Options::default();
                opt.resources_dir = std::fs::canonicalize(&path)
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()));
                opt.fontdb_mut().load_system_fonts();

                let svg_data =
                    std::fs::read(&path).map_err(|e| format!("{} svg read failed: {}", path, e))?;
                usvg::Tree::from_data(&svg_data, &opt)
                    .map_err(|e| format!("{} svg parse failed: {}", path, e))?
            };

            let mut pixmap_size = tree.size().to_int_size();
            let scale_factor = max_dimension as f32
                / std::cmp::max(pixmap_size.width(), pixmap_size.height()) as f32;
            if scale_factor < 1.0 {
                let (nwidth, nheight) = (
                    pixmap_size.width() as f32 * scale_factor,
                    pixmap_size.height() as f32 * scale_factor,
                );
                pixmap_size = tiny_skia::IntSize::from_wh(nwidth as u32, nheight as u32)
                    .ok_or_else(|| format!("{} invalid svg dimensions", path))?;
            }
            let mut pixmap = tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height())
                .ok_or_else(|| format!("{} pixmap creation failed", path))?;

            resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
            pixmap
                .encode_png()
                .map_err(|_| format!("{} encode_png failed", path))
        }
        _ => Err(format!(
            "Unsupported image format (extension): {}",
            extension
        )),
    }?;

    #[allow(deprecated)]
    let m_content = base64::encode(&data);

    MultimodalElement::new(f_type.clone(), m_content)
}

fn cat_resolved_path_key(path: &str) -> String {
    refact_core::chat_types::normalize_file_name(path.to_string())
}

fn rebuild_cat_seen_by_path(
    resolved_paths: &[CatResolvedPath],
    seen_by_path: &mut HashMap<String, Vec<usize>>,
) {
    seen_by_path.clear();
    for (index, resolved_path) in resolved_paths.iter().enumerate() {
        seen_by_path
            .entry(cat_resolved_path_key(&resolved_path.path))
            .or_default()
            .push(index);
    }
}

fn remove_cat_resolved_paths(
    resolved_paths: &mut Vec<CatResolvedPath>,
    seen_by_path: &mut HashMap<String, Vec<usize>>,
    indices_to_remove: HashSet<usize>,
) {
    if indices_to_remove.is_empty() {
        return;
    }

    let mut index = 0;
    resolved_paths.retain(|_| {
        let keep = !indices_to_remove.contains(&index);
        index += 1;
        keep
    });
    rebuild_cat_seen_by_path(resolved_paths, seen_by_path);
}

fn push_cat_resolved_path(
    resolved_paths: &mut Vec<CatResolvedPath>,
    seen_by_path: &mut HashMap<String, Vec<usize>>,
    incoming: CatResolvedPath,
) {
    let key = cat_resolved_path_key(&incoming.path);
    if let Some(indices) = seen_by_path.get(&key).cloned() {
        let has_explicit = indices
            .iter()
            .any(|index| resolved_paths[*index].source == CatResolvedSource::ExplicitFile);
        if incoming.source == CatResolvedSource::DirectoryExpansion && has_explicit {
            return;
        }

        if incoming.source == CatResolvedSource::ExplicitFile && !has_explicit {
            let indices_to_remove = indices
                .iter()
                .copied()
                .filter(|index| {
                    resolved_paths[*index].source == CatResolvedSource::DirectoryExpansion
                })
                .collect::<HashSet<_>>();
            remove_cat_resolved_paths(resolved_paths, seen_by_path, indices_to_remove);
            let index = resolved_paths.len();
            resolved_paths.push(incoming);
            seen_by_path.entry(key).or_default().push(index);
            return;
        }

        for index in &indices {
            let existing = &mut resolved_paths[*index];
            if existing.line_range == incoming.line_range {
                if existing.source == CatResolvedSource::DirectoryExpansion
                    && incoming.source == CatResolvedSource::ExplicitFile
                {
                    existing.source = CatResolvedSource::ExplicitFile;
                }
                return;
            }
        }

        if let Some(incoming_range) = incoming.line_range {
            for index in &indices {
                let existing = &mut resolved_paths[*index];
                if existing.line_range.is_none() {
                    existing.line_range = Some(incoming_range);
                    if incoming.source == CatResolvedSource::ExplicitFile {
                        existing.source = CatResolvedSource::ExplicitFile;
                    }
                    return;
                }
            }
        } else if indices
            .iter()
            .any(|index| resolved_paths[*index].line_range.is_some())
        {
            return;
        }
    }

    let index = resolved_paths.len();
    resolved_paths.push(incoming);
    seen_by_path.entry(key).or_default().push(index);
}

fn push_cat_resolved_file(
    resolved_paths: &mut Vec<CatResolvedPath>,
    seen_by_path: &mut HashMap<String, Vec<usize>>,
    path: String,
    line_range: Option<CatLineRange>,
    source: CatResolvedSource,
) {
    push_cat_resolved_path(
        resolved_paths,
        seen_by_path,
        CatResolvedPath {
            path,
            line_range,
            source,
        },
    );
}

async fn paths_and_symbols_to_cat_with_path_ranges(
    ccx: Arc<AMutex<AtCommandsContext>>,
    paths: Vec<CatPathRequest>,
    arg_symbols: Vec<String>,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<ContextEnum>,
    Vec<MultimodalElement>,
    Vec<String>,
) {
    let (gcx, top_n, execution_scope) = {
        let cgcx = ccx.lock().await;
        (
            cgcx.app.gcx.clone(),
            cgcx.top_n,
            cgcx.execution_scope.clone(),
        )
    };
    let mut not_found_messages = vec![];
    let mut scope_notices = vec![];
    let mut resolved_paths = vec![];
    let mut seen_by_path = HashMap::new();

    for request in paths {
        let line_range = request.line_range;
        let p = request.path;
        if execution_scope
            .as_ref()
            .map(|scope| scope.is_enforced())
            .unwrap_or(false)
        {
            match resolve_existing_path_with_execution_scope(
                gcx.clone(),
                execution_scope.as_ref(),
                &p,
            )
            .await
            {
                Ok(Some(resolved)) => {
                    scope_notices.extend(resolved.notices);
                    if resolved.path.is_dir() {
                        match list_scoped_files_under_dir(gcx.clone(), &resolved.path, false, true)
                            .await
                        {
                            Ok(files_in_dir) => {
                                for file in files_in_dir {
                                    let file_str = file.to_string_lossy().to_string();
                                    push_cat_resolved_file(
                                        &mut resolved_paths,
                                        &mut seen_by_path,
                                        file_str,
                                        line_range,
                                        CatResolvedSource::DirectoryExpansion,
                                    );
                                }
                            }
                            Err(e) => not_found_messages.push(e),
                        }
                    } else if resolved.path.is_file() {
                        let file_str = resolved.path.to_string_lossy().to_string();
                        push_cat_resolved_file(
                            &mut resolved_paths,
                            &mut seen_by_path,
                            file_str,
                            line_range,
                            CatResolvedSource::ExplicitFile,
                        );
                    } else {
                        not_found_messages.push(format!(
                            "Path '{}' is not a file or directory",
                            resolved.path.display()
                        ));
                    }
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    not_found_messages.push(e);
                    continue;
                }
            }
        }

        let path = if PathBuf::from(&p).is_absolute() {
            canonical_path(p).to_string_lossy().to_string()
        } else {
            preprocess_path_for_normalization(p)
        };

        let candidates_file = file_repair_candidates(gcx.clone(), &path, top_n, false).await;
        let candidates_dir = correct_to_nearest_dir_path(gcx.clone(), &path, false, top_n).await;

        if !candidates_file.is_empty() || candidates_dir.is_empty() {
            let file_path = match return_one_candidate_or_a_good_error(
                gcx.clone(),
                &path,
                &candidates_file,
                &get_project_dirs(gcx.clone()).await,
                false,
            )
            .await
            {
                Ok(f) => f,
                Err(e) => {
                    not_found_messages.push(e);
                    continue;
                }
            };
            push_cat_resolved_file(
                &mut resolved_paths,
                &mut seen_by_path,
                file_path,
                line_range,
                CatResolvedSource::ExplicitFile,
            );
        } else {
            let candidate = match return_one_candidate_or_a_good_error(
                gcx.clone(),
                &path,
                &candidates_dir,
                &get_project_dirs(gcx.clone()).await,
                true,
            )
            .await
            {
                Ok(f) => f,
                Err(e) => {
                    not_found_messages.push(e);
                    continue;
                }
            };
            let path_buf = PathBuf::from(candidate);
            let indexing_everywhere =
                crate::files_blocklist::reload_indexing_everywhere_if_needed(gcx.clone()).await;
            let files_in_dir = ls_files(&indexing_everywhere, &path_buf, false).unwrap_or(vec![]);
            for file in files_in_dir {
                let file_str = file.to_string_lossy().to_string();
                push_cat_resolved_file(
                    &mut resolved_paths,
                    &mut seen_by_path,
                    file_str,
                    line_range,
                    CatResolvedSource::DirectoryExpansion,
                );
            }
        }
    }

    let mut context_enums = vec![];
    let mut symbols_found = HashSet::<String>::new();
    let mut symbols_not_found = vec![];
    let mut filenames_present = vec![];
    let mut multimodal: Vec<MultimodalElement> = vec![];

    let codegraph_opt = gcx.codegraph.lock().await.clone();
    if let Some(service) = &codegraph_opt {
        for request in resolved_paths.iter() {
            let p = &request.path;
            let line_range = request.line_range;

            let doc_syms = service.doc_defs(p).await.unwrap_or_default();
            // s.name() means the last part of the path
            // symbols.contains means exact match in comma-separated list
            let mut syms_def_in_this_file = vec![];
            for looking_for in arg_symbols.iter() {
                let colon_colon_looking_for = format!("::{}", looking_for.trim());
                let mut found_in_this_file = false;
                for x in doc_syms.iter() {
                    if x.path().ends_with(colon_colon_looking_for.as_str()) {
                        syms_def_in_this_file.push(x.clone());
                        found_in_this_file = true;
                    }
                }
                if found_in_this_file {
                    symbols_found.insert(looking_for.clone());
                }
            }

            for sym in syms_def_in_this_file {
                let sym_start = sym.full_line1();
                let sym_end = sym.full_line2();

                // If line range is specified, check overlap
                let (start_line, end_line) = match line_range {
                    Some((start, end)) => {
                        // If symbol doesn't overlap with requested line range, skip it
                        if end < sym_start || start > sym_end {
                            // Symbol is completely outside requested range
                            continue;
                        }
                        // Show the intersection of symbol range and requested range
                        (start.max(sym_start), end.min(sym_end))
                    }
                    None => (sym_start, sym_end),
                };

                let cf = ContextFile {
                    file_name: refact_core::chat_types::normalize_file_name(p.clone()),
                    file_content: "".to_string(),
                    line1: start_line,
                    line2: end_line,
                    file_rev: None,
                    symbols: vec![sym.path_drop0()],
                    gradient_type: 5,
                    usefulness: 100.0,
                    skip_pp: true,
                };
                context_enums.push(ContextEnum::ContextFile(cf));
            }
        }
    }

    for looking_for in arg_symbols.iter() {
        if !symbols_found.contains(looking_for) {
            symbols_not_found.push(looking_for.clone());
        }
    }

    let filenames_got_symbols_for = context_enums
        .iter()
        .filter_map(|x| {
            if let ContextEnum::ContextFile(cf) = x {
                Some(cf.file_name.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut image_counter = 0;
    for request in resolved_paths
        .iter()
        .filter(|request| !filenames_got_symbols_for.contains(&request.path))
    {
        let p = &request.path;
        let line_range = request.line_range;

        let path_buf = PathBuf::from(p);
        if let Err(e) = check_file_privacy_for_send(gcx.clone(), &path_buf).await {
            not_found_messages.push(format!("{}: {}", p, e));
            continue;
        }

        // don't have symbols for these, so we need to mention them as files, without a symbol, analog of @file
        let f_type = get_file_type(&path_buf);

        if f_type.starts_with("image/") {
            filenames_present.push(p.clone());
            image_counter += 1;
            if image_counter > CAT_MAX_IMAGES_CNT {
                if image_counter == CAT_MAX_IMAGES_CNT + 1 {
                    not_found_messages.push(format!("⚠️ showing 1 of {} images (limit: 1). 💡 Call cat() separately for each image", resolved_paths.iter().filter(|request| get_file_type(&PathBuf::from(&request.path)).starts_with("image/")).count()));
                }
                continue;
            }
            match load_image(p, &f_type).await {
                Ok(mm) => {
                    multimodal.push(mm);
                }
                Err(e) => {
                    not_found_messages.push(format!("{}: {}", p, e));
                }
            }
        } else {
            match get_file_text_from_memory_or_disk(gcx.clone(), &path_buf).await {
                Ok(text) => {
                    let total_lines = text.lines().count();
                    let (start_line, end_line) = match line_range {
                        Some((start, end)) => {
                            let start = start.max(1);
                            let end = end.min(total_lines).max(start);
                            if start > total_lines {
                                not_found_messages.push(format!(
                                    "⚠️ line {} is beyond file end ({} lines). 💡 Use cat('{}:1-{}')",
                                    start, total_lines, p, total_lines
                                ));
                                (1, total_lines.min(CAT_MAX_LINES))
                            } else {
                                (start, end)
                            }
                        }
                        None => {
                            if total_lines > CAT_MAX_LINES {
                                not_found_messages.push(format!(
                                    "⚠️ {} has {} lines, showing first {} lines. 💡 Use cat('{}:START-END') to read specific line ranges",
                                    p, total_lines, CAT_MAX_LINES, p
                                ));
                            }
                            (1, total_lines.min(CAT_MAX_LINES))
                        }
                    };

                    let cf = ContextFile {
                        file_name: refact_core::chat_types::normalize_file_name(p.clone()),
                        file_content: "".to_string(),
                        line1: start_line,
                        line2: end_line,
                        file_rev: None,
                        symbols: vec![],
                        gradient_type: 5,
                        usefulness: 100.0,
                        skip_pp: true,
                    };
                    context_enums.push(ContextEnum::ContextFile(cf));
                }
                Err(e) => {
                    not_found_messages.push(format!("{}: {}", p, e));
                }
            }
        }
    }
    for cf in context_enums.iter().filter_map(|x| {
        if let ContextEnum::ContextFile(cf) = x {
            Some(cf)
        } else {
            None
        }
    }) {
        filenames_present.push(cf.file_name.clone());
    }
    (
        filenames_present,
        symbols_not_found,
        not_found_messages,
        context_enums,
        multimodal,
        scope_notices,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::privacy::{FilePrivacySettings, PrivacySettings};

    async fn ccx_for_root(root: &std::path::Path) -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![canonical_path(root.to_string_lossy())];
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec![],
                blocked: vec![],
            },
            loaded_ts: u64::MAX / 2,
        });
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "test-chat".to_string(),
                None,
                "test-model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn cat_args(paths: String) -> HashMap<String, Value> {
        HashMap::from_iter([("paths".to_string(), Value::String(paths))])
    }

    fn normalized(path: &std::path::Path) -> String {
        refact_core::chat_types::normalize_file_name(path.to_string_lossy().to_string())
    }

    fn context_file_ranges(results: &[ContextEnum]) -> Vec<(String, usize, usize)> {
        results
            .iter()
            .filter_map(|item| match item {
                ContextEnum::ContextFile(file) => {
                    Some((file.file_name.clone(), file.line1, file.line2))
                }
                _ => None,
            })
            .collect()
    }

    fn tool_text(results: &[ContextEnum]) -> String {
        results
            .iter()
            .filter_map(|item| match item {
                ContextEnum::ChatMessage(message) => match &message.content {
                    ChatContent::SimpleText(text) => Some(text.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn write_lines(path: &std::path::Path, lines: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = (1..=lines)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, format!("{content}\n")).unwrap();
    }

    async fn run_cat(ccx: Arc<AMutex<AtCommandsContext>>, paths: String) -> Vec<ContextEnum> {
        let mut tool = ToolCat {
            config_path: String::new(),
        };
        let (_, results) = tool
            .tool_execute(ccx, &"cat-call".to_string(), &cat_args(paths))
            .await
            .unwrap();
        results
    }

    #[tokio::test]
    async fn tool_cat_duplicate_path_keeps_explicit_range_before_unbounded() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("f.rs");
        write_lines(&file, 8);
        let ccx = ccx_for_root(temp.path()).await;

        let results = run_cat(
            ccx,
            format!("{}:2-4,{}", file.to_string_lossy(), file.to_string_lossy()),
        )
        .await;

        assert_eq!(
            context_file_ranges(&results),
            vec![(normalized(&file), 2, 4)],
            "{}",
            tool_text(&results)
        );
    }

    #[tokio::test]
    async fn tool_cat_duplicate_explicit_ranges_remain_distinct_and_ordered() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("f.rs");
        write_lines(&file, 8);
        let ccx = ccx_for_root(temp.path()).await;

        let results = run_cat(
            ccx,
            format!(
                "{}:2-3,{}:6-7",
                file.to_string_lossy(),
                file.to_string_lossy()
            ),
        )
        .await;

        assert_eq!(
            context_file_ranges(&results),
            vec![(normalized(&file), 2, 3), (normalized(&file), 6, 7)]
        );
    }

    #[tokio::test]
    async fn tool_cat_explicit_file_range_wins_over_directory_expansion() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let dir = temp.path().join("src");
        let other = dir.join("a.rs");
        let file = dir.join("z.rs");
        write_lines(&other, 4);
        write_lines(&file, 8);
        let ccx = ccx_for_root(temp.path()).await;

        let results = run_cat(
            ccx,
            format!("{}:3-5,{}", file.to_string_lossy(), dir.to_string_lossy()),
        )
        .await;

        assert_eq!(
            context_file_ranges(&results),
            vec![(normalized(&file), 3, 5), (normalized(&other), 1, 4)]
        );
    }

    #[tokio::test]
    async fn tool_cat_later_explicit_file_range_moves_after_directory_expansion() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let dir = temp.path().join("src");
        let file = dir.join("a.rs");
        let other = dir.join("z.rs");
        write_lines(&file, 8);
        write_lines(&other, 4);
        let ccx = ccx_for_root(temp.path()).await;

        let results = run_cat(
            ccx,
            format!("{},{}:3-5", dir.to_string_lossy(), file.to_string_lossy()),
        )
        .await;

        assert_eq!(
            context_file_ranges(&results),
            vec![(normalized(&other), 1, 4), (normalized(&file), 3, 5)]
        );
    }
}
