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
    check_file_privacy_for_send, get_file_text_from_memory_or_disk_with_context, ls_files_limited,
    prepare_file_read_context,
};
use crate::scratchpads::multimodality::MultimodalElement;
use crate::knowledge_index::format_related_memories_section;
use crate::tools::scope_utils::{
    format_scope_notices, list_scoped_files_under_dir_limited,
    resolve_existing_path_with_execution_scope,
};

use refact_core::image_policy::{resize_to_policy, ImagePolicy};

pub struct ToolCat {
    pub config_path: String,
}

const CAT_MAX_IMAGES_CNT: usize = 10;
const CAT_MAX_LINES: usize = 2000;
const CAT_MAX_INPUT_PATHS: usize = 128;
const CAT_MAX_EXPANDED_FILES: usize = 512;
const CAT_MAX_RANGE_SPAN: usize = CAT_MAX_LINES;
const CAT_MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const CAT_MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

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

    if paths.len() > CAT_MAX_INPUT_PATHS {
        paths.truncate(CAT_MAX_INPUT_PATHS);
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

        let mut tool_message = ChatMessage {
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
        };
        let gcx = ccx.lock().await.app.gcx.clone();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let records = crate::privacy::records::declared_file_records(
            &gcx,
            filenames_present.iter().map(PathBuf::from),
        )?;
        crate::privacy::records::merge_records(&mut tool_message, records);
        results.push(ContextEnum::ChatMessage(tool_message));

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

async fn load_image(
    path: &String,
    f_type: &String,
    image_policy: &ImagePolicy,
) -> Result<MultimodalElement, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| format!("{} image metadata failed: {}", path, error))?;
    if metadata.len() > CAT_MAX_IMAGE_BYTES {
        return Err(format!(
            "{} image exceeds the {} byte input limit",
            path, CAT_MAX_IMAGE_BYTES
        ));
    }
    let extension = path.split(".").last().unwrap().to_string();
    let mut f_type = f_type.clone();

    let data = match f_type.as_str() {
        "image/png" | "image/jpeg" => {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|_| format!("{} image read failed", path))?;
            let (data, resized_mime) = resize_to_policy(&bytes, &f_type, image_policy)
                .map_err(|error| format!("{} {}", path, error.to_ascii_lowercase()))?;
            f_type = resized_mime;
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
            let scale_factor = image_policy.preferred_side.min(image_policy.max_side) as f32
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

fn clamp_cat_line_range(line_range: Option<CatLineRange>) -> Option<CatLineRange> {
    line_range.map(|(start, end)| {
        let start = start.max(1);
        let max_end = start.saturating_add(CAT_MAX_RANGE_SPAN.saturating_sub(1));
        (start, end.min(max_end))
    })
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
    let (gcx, top_n, execution_scope, abort_flag, current_model) = {
        let cgcx = ccx.lock().await;
        (
            cgcx.app.gcx.clone(),
            cgcx.top_n,
            cgcx.execution_scope.clone(),
            cgcx.abort_flag.clone(),
            cgcx.current_model.clone(),
        )
    };
    let image_policy =
        match crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await {
            Ok(caps) => crate::caps::resolve_chat_model(caps, &current_model)
                .map(|model| ImagePolicy::for_model(&model.base))
                .unwrap_or_default(),
            Err(_) => ImagePolicy::default(),
        };
    let aborted = || abort_flag.load(std::sync::atomic::Ordering::Relaxed);
    let mut not_found_messages = vec![];
    let mut scope_notices = vec![];
    let mut resolved_paths = vec![];
    let mut seen_by_path = HashMap::new();
    let mut expanded_files_count: usize = 0;
    let mut expansion_capped = false;

    for request in paths {
        if aborted() {
            break;
        }
        let line_range = clamp_cat_line_range(request.line_range);
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
                        let remaining = CAT_MAX_EXPANDED_FILES.saturating_sub(expanded_files_count);
                        match list_scoped_files_under_dir_limited(
                            gcx.clone(),
                            &resolved.path,
                            false,
                            true,
                            remaining.saturating_add(1),
                            Some(&abort_flag),
                        )
                        .await
                        {
                            Ok(listing) => {
                                expansion_capped |= listing.truncated;
                                for file in listing.files {
                                    if expanded_files_count >= CAT_MAX_EXPANDED_FILES {
                                        expansion_capped = true;
                                        break;
                                    }
                                    let file_str = file.to_string_lossy().to_string();
                                    push_cat_resolved_file(
                                        &mut resolved_paths,
                                        &mut seen_by_path,
                                        file_str,
                                        line_range,
                                        CatResolvedSource::DirectoryExpansion,
                                    );
                                    expanded_files_count += 1;
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
            let remaining = CAT_MAX_EXPANDED_FILES.saturating_sub(expanded_files_count);
            let listing = ls_files_limited(
                &indexing_everywhere,
                &path_buf,
                false,
                remaining.saturating_add(1),
                Some(&abort_flag),
            )
            .unwrap_or_default();
            expansion_capped |= listing.truncated;
            for file in listing.files {
                if expanded_files_count >= CAT_MAX_EXPANDED_FILES {
                    expansion_capped = true;
                    break;
                }
                let file_str = file.to_string_lossy().to_string();
                push_cat_resolved_file(
                    &mut resolved_paths,
                    &mut seen_by_path,
                    file_str,
                    line_range,
                    CatResolvedSource::DirectoryExpansion,
                );
                expanded_files_count += 1;
            }
        }
    }

    if expansion_capped {
        not_found_messages.push(format!(
            "⚠️ directory expansion produced more than {} files, showing the first {}. 💡 Narrow the path or cat() specific files.",
            CAT_MAX_EXPANDED_FILES, CAT_MAX_EXPANDED_FILES
        ));
    }

    {
        let mut allowed_paths: Vec<CatResolvedPath> = Vec::with_capacity(resolved_paths.len());
        for request in resolved_paths.into_iter() {
            let path_buf = PathBuf::from(&request.path);
            if check_file_privacy_for_send(gcx.clone(), &path_buf)
                .await
                .is_err()
            {
                continue;
            }
            allowed_paths.push(request);
        }
        resolved_paths = allowed_paths;
        rebuild_cat_seen_by_path(&resolved_paths, &mut seen_by_path);
    }

    let mut context_enums = vec![];
    let mut symbols_found = HashSet::<String>::new();
    let mut symbols_not_found = vec![];
    let mut filenames_present = vec![];
    let mut multimodal: Vec<MultimodalElement> = vec![];

    let codegraph_opt = if aborted() {
        None
    } else {
        gcx.codegraph.lock().await.clone()
    };
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

    let read_context = prepare_file_read_context(gcx.clone()).await;
    let mut image_counter = 0;
    for request in resolved_paths
        .iter()
        .filter(|request| !filenames_got_symbols_for.contains(&request.path))
    {
        if aborted() {
            break;
        }
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
                    not_found_messages.push(format!("⚠️ showing {} of {} images (limit: {}). 💡 Call cat() separately for remaining images", CAT_MAX_IMAGES_CNT, resolved_paths.iter().filter(|request| get_file_type(&PathBuf::from(&request.path)).starts_with("image/")).count(), CAT_MAX_IMAGES_CNT));
                }
                continue;
            }
            match load_image(p, &f_type, &image_policy).await {
                Ok(mm) => {
                    multimodal.push(mm);
                }
                Err(e) => {
                    not_found_messages.push(format!("{}: {}", p, e));
                }
            }
        } else {
            match get_file_text_from_memory_or_disk_with_context(
                gcx.clone(),
                &path_buf,
                &read_context,
                Some(CAT_MAX_FILE_BYTES),
            )
            .await
            {
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
                    if e.contains("byte search limit") {
                        filenames_present
                            .push(refact_core::chat_types::normalize_file_name(p.clone()));
                        not_found_messages.push(format!(
                            "⚠️ {} exceeds the {} byte read limit and was skipped. 💡 Use cat('{}:START-END') to read a specific line range.",
                            p, CAT_MAX_FILE_BYTES, p
                        ));
                    } else {
                        not_found_messages.push(format!("{}: {}", p, e));
                    }
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
        ccx_for_root_with_blocked(root, vec![]).await
    }

    async fn ccx_for_root_with_blocked(
        root: &std::path::Path,
        blocked: Vec<String>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![canonical_path(root.to_string_lossy())];
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec![],
                blocked,
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
        // Mirror the production path pipeline: canonical_path resolves platform
        // quirks (Windows 8.3 short names like RUNNER~1, symlinked /tmp on macOS)
        // exactly like tool_cat does before emitting context files.
        let canonical = crate::files_correction::canonical_path(path.to_string_lossy().to_string());
        refact_core::chat_types::normalize_file_name(canonical.to_string_lossy().to_string())
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

    #[tokio::test]
    async fn tool_cat_blocked_file_is_never_disclosed() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let dir = temp.path().join("src");
        let visible = dir.join("visible.rs");
        let secret = dir.join("secret.rs");
        write_lines(&visible, 4);
        write_lines(&secret, 4);
        let ccx = ccx_for_root_with_blocked(temp.path(), vec!["*/secret.rs".to_string()]).await;

        let results = run_cat(
            ccx,
            format!("{},{}", dir.to_string_lossy(), secret.to_string_lossy()),
        )
        .await;

        let secret_norm = normalized(&secret);
        assert!(
            context_file_ranges(&results)
                .iter()
                .all(|(name, _, _)| name != &secret_norm),
            "blocked file leaked as a context file: {}",
            tool_text(&results)
        );
        let text = tool_text(&results);
        assert!(
            !text.contains("secret.rs"),
            "blocked file path leaked in text output: {}",
            text
        );
        let visible_norm = normalized(&visible);
        assert!(
            context_file_ranges(&results)
                .iter()
                .any(|(name, _, _)| name == &visible_norm),
            "visible file was unexpectedly dropped: {}",
            text
        );
    }

    #[tokio::test]
    async fn tool_cat_directory_expansion_is_bounded() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let dir = temp.path().join("many");
        std::fs::create_dir_all(&dir).unwrap();
        let total = CAT_MAX_EXPANDED_FILES + 5;
        for i in 0..total {
            let f = dir.join(format!("f{:04}.rs", i));
            std::fs::write(&f, "line 1\n").unwrap();
        }
        let ccx = ccx_for_root(temp.path()).await;

        let results = run_cat(ccx, dir.to_string_lossy().to_string()).await;

        let emitted = context_file_ranges(&results).len();
        assert!(
            emitted <= CAT_MAX_EXPANDED_FILES,
            "directory expansion was not bounded: emitted {} > cap {}",
            emitted,
            CAT_MAX_EXPANDED_FILES
        );
        let text = tool_text(&results);
        assert!(
            text.contains("directory expansion produced more than"),
            "expected an expansion-cap notice, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn tool_cat_large_file_is_bounded() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let big = temp.path().join("big.rs");
        std::fs::create_dir_all(big.parent().unwrap()).unwrap();
        let chunk = "x".repeat(1024);
        let mut content = String::with_capacity(CAT_MAX_FILE_BYTES + 4096);
        while content.len() <= CAT_MAX_FILE_BYTES + 2048 {
            content.push_str(&chunk);
            content.push('\n');
        }
        std::fs::write(&big, content).unwrap();
        let ccx = ccx_for_root(temp.path()).await;

        let results = run_cat(ccx, big.to_string_lossy().to_string()).await;

        let big_norm = normalized(&big);
        assert!(
            context_file_ranges(&results)
                .iter()
                .all(|(name, _, _)| name != &big_norm),
            "oversized file was emitted despite the byte cap: {}",
            tool_text(&results)
        );
        let text = tool_text(&results);
        assert!(
            text.contains("exceeds the") && text.contains("byte read limit"),
            "expected an oversized-file notice, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn tool_cat_attaches_declared_secrets_record() {
        let temp = tempfile::Builder::new()
            .prefix("refact-tool-cat-")
            .tempdir()
            .unwrap();
        let file = temp.path().join(".env");
        write_lines(&file, 2);
        let ccx = ccx_for_root(temp.path()).await;
        let gcx = ccx.lock().await.app.gcx.clone();
        let policy = refact_privacy::PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                refact_privacy::Zone {
                    name: "secrets".to_string(),
                    patterns: vec![file.to_string_lossy().to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                },
                refact_privacy::Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                },
            ],
            subagents: refact_privacy::SubagentPolicy::default(),
        };
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(policy);

        let results = run_cat(ccx, file.to_string_lossy().to_string()).await;
        let message = results
            .iter()
            .find_map(|result| match result {
                ContextEnum::ChatMessage(message) if message.role == "tool" => Some(message),
                _ => None,
            })
            .expect("cat should emit a tool message");

        assert_eq!(message.extra["privacy"]["files"][0]["zone"], "secrets");
        assert_eq!(
            message.extra["privacy"]["files"][0]["attribution"],
            "declared"
        );
    }
}
