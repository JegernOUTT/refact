use super::*;
use refact_chat_api::PythonEngineClass as Class;

fn project(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("project")
}

/// The approval request of a spawned run; when the run ends without asking, fails with its output.
async fn ask(
    app: &AppState,
    code: &str,
) -> (
    tokio::task::JoinHandle<ChatMessage>,
    refact_chat_api::PythonApprovalRequest,
) {
    let task = spawn(app, code);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Some(request) = session(app)
            .await
            .lock()
            .await
            .runtime
            .python_approvals
            .first()
            .cloned()
        {
            return (task, request);
        }
        if task.is_finished() {
            let m = finish(task).await;
            panic!("{code}: ended without asking: {:?}", m.extra["python"]);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{code}: no approval request"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

async fn approve(
    app: &AppState,
    code: &str,
) -> (refact_chat_api::PythonApprovalRequest, ChatMessage) {
    let (task, request) = ask(app, code).await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    (request, finish(task).await)
}

async fn denied(app: &AppState, code: &str) -> refact_chat_api::PythonApprovalRequest {
    let (task, request) = ask(app, code).await;
    decide(app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    request
}

/// Raises the card, runs `mutate` before the answer, allows, and returns the error text.
async fn moved<F: std::future::Future<Output = ()>>(
    app: &AppState,
    call: &str,
    mutate: impl FnOnce() -> F,
) -> String {
    let (task, request) = ask(app, &format!("r = await {call}\n[r['ok'], r['error']]")).await;
    mutate().await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{call}: {out}");
    out[1].as_str().unwrap_or_default().to_string()
}

fn error_type(m: &ChatMessage) -> Value {
    m.extra["python"]["error"]["type"].clone()
}

/// A refusal before any card: the error type, with no approval left behind.
async fn refused(app: &AppState, code: &str) -> String {
    let m = run(app, code).await;
    assert!(
        session(app)
            .await
            .lock()
            .await
            .runtime
            .python_approvals
            .is_empty(),
        "a card was raised"
    );
    error_type(&m).as_str().unwrap_or_default().to_string()
}

fn engine(request: refact_chat_api::PythonApprovalRequest) -> refact_chat_api::PythonEngineRequest {
    request.engine.expect("an engine card")
}

fn item<'a>(
    engine: &'a refact_chat_api::PythonEngineRequest,
    label: &str,
) -> &'a refact_chat_api::PythonEngineItem {
    engine
        .items
        .iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("no item {label} in {:?}", engine.items))
}

fn provider_file(app: &AppState, name: &str) -> String {
    std::fs::read_to_string(app.gcx.config_dir.join(format!("providers.d/{name}.yaml")))
        .unwrap_or_default()
}

async fn create_provider(app: &AppState, name: &str, body: Value) {
    crate::providers::http::update_provider_settings(
        app.gcx.clone(),
        name,
        serde_yaml::to_value(body).unwrap(),
    )
    .await
    .unwrap();
}

async fn custom_provider(app: &AppState) {
    create_provider(
        app,
        "custom_2",
        json!({"base_provider":"custom","api_key":"sk-zone-old","chat_endpoint":"https://a.zone.example/v1/chat/completions","enabled":true}),
    )
    .await;
}

#[tokio::test]
async fn providers_set_endpoint_change_is_risky_and_keeps_the_key() {
    let (_dir, app, _) = fixture().await;
    custom_provider(&app).await;
    let call = "engine('providers.set', provider='custom_2', settings={'chat_endpoint': 'https://b.zone.example/v1/chat/completions'})";
    let request = denied(&app, &format!("await {call}")).await;
    assert!(!serde_json::to_string(&request)
        .unwrap()
        .contains("sk-zone-old"));
    let card = engine(request);
    assert_eq!(card.class, Class::Risky);
    assert_eq!(card.title, "change provider settings");
    assert!(card.warning.clone().unwrap().contains("different server"));
    assert!(card.scope.clone().unwrap().contains("custom_2"));
    let endpoint = item(&card, "chat_endpoint");
    assert!(endpoint
        .old
        .as_ref()
        .unwrap()
        .to_string()
        .contains("a.zone.example"));
    assert!(endpoint
        .new
        .as_ref()
        .unwrap()
        .to_string()
        .contains("b.zone.example"));
    assert!(provider_file(&app, "custom_2").contains("a.zone.example"));
    let (_, m) = approve(&app, &format!("(await {call})['data']")).await;
    assert_eq!(result(&m)["provider"], "custom_2", "{}", result(&m));
    let saved = provider_file(&app, "custom_2");
    assert!(
        saved.contains("b.zone.example") && saved.contains("sk-zone-old"),
        "{saved}"
    );
    let (request, _) = approve(
        &app,
        "await engine('providers.set', provider='custom_2', settings={'supports_cache_control': True})",
    )
    .await;
    let card = engine(request);
    assert_eq!(card.class, Class::Change);
    assert!(card.warning.is_none());
    let same = result(&run(&app, "(await engine('providers.set', provider='custom_2', settings={'supports_cache_control': True}))['text']").await);
    assert_eq!(same, "nothing to change");
}

#[tokio::test]
async fn providers_set_is_a_change_only_for_listed_keys_and_plain_values() {
    let (_dir, app, _) = fixture().await;
    custom_provider(&app).await;
    for settings in [
        "{'mcp_gateway_url': 'https://zone.attacker.example'}",
        "{'foundry_base_url': 'https://zone.attacker.example/anthropic'}",
        "{'usage_endpoint': 'zone.attacker.example'}",
        "{'backend': 'custom'}",
        "{'region': 'zone-west-1'}",
        "{'enabled': 'https://zone.attacker.example'}",
    ] {
        let card = engine(
            denied(
                &app,
                &format!("await engine('providers.set', provider='custom_2', settings={settings})"),
            )
            .await,
        );
        assert_eq!(card.class, Class::Risky, "{settings}");
        assert!(card.warning.is_some(), "{settings}");
    }
    for settings in [
        "{'enabled': False}",
        "{'wire_format': 'zone-format'}",
        "{'supports_cache_control': True}",
        "{'zone_max_retries': 3}",
        "{'zone_flag': True}",
    ] {
        let card = engine(
            denied(
                &app,
                &format!("await engine('providers.set', provider='custom_2', settings={settings})"),
            )
            .await,
        );
        assert_eq!(card.class, Class::Change, "{settings}");
        assert!(card.warning.is_none(), "{settings}");
    }
    assert_eq!(
        refused(
            &app,
            "await engine('providers.set', provider='custom_2', settings={'azure_resource': '$ZONE_SECRET_ENV'})"
        )
        .await,
        "PermissionError"
    );
}

#[tokio::test]
async fn providers_set_refuses_secrets_and_identity_before_any_card() {
    let (_dir, app, _) = fixture().await;
    custom_provider(&app).await;
    let before = provider_file(&app, "custom_2");
    for settings in [
        "{'api_key': 'sk-zone-new'}",
        "{'auth_token': 'x'}",
        "{'tokenizer_api_key': 'x'}",
        "{'extra_headers': {'X-Zone': 'y'}}",
        "{'credential': {'type': 'command', 'command': 'zone-helper'}}",
        "{'oauth_tokens': {'access_token': 'x'}}",
        "{'aws_access_key_id': 'x'}",
        "{'base_provider': 'openai'}",
        "{'display_name': 'Zone'}",
        "{'chat_endpoint': '***'}",
        "{'zone_options': {'password': 'x'}}",
    ] {
        let kind = refused(
            &app,
            &format!("await engine('providers.set', provider='custom_2', settings={settings})"),
        )
        .await;
        assert_eq!(kind, "PermissionError", "{settings}");
    }
    assert_eq!(provider_file(&app, "custom_2"), before);
    assert_eq!(
        refused(
            &app,
            "await engine('providers.set', provider='zone_missing', settings={'enabled': False})"
        )
        .await,
        "TypeError"
    );
}

#[tokio::test]
async fn providers_set_refuses_when_the_file_moved_after_the_card() {
    let (_dir, app, _) = fixture().await;
    custom_provider(&app).await;
    let app2 = app.clone();
    let error = moved(
        &app,
        "engine('providers.set', provider='custom_2', settings={'chat_endpoint': 'https://b.zone.example/v1'})",
        || async move {
            create_provider(&app2, "custom_2", json!({"chat_endpoint":"https://c.zone.example/v1"})).await;
        },
    )
    .await;
    assert!(error.contains("changed since the preview"), "{error}");
    assert!(provider_file(&app, "custom_2").contains("c.zone.example"));
}

fn copilot_file(app: &AppState, token: &str) {
    let dir = app.gcx.config_dir.join("providers.d");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("github_copilot.yaml"),
        format!("oauth_tokens:\n  access_token: {token}\n  expires_at: 0\n  api_base: https://api.githubcopilot.com\n"),
    )
    .unwrap();
}

#[tokio::test]
async fn providers_sign_out_is_destructive_and_clears_the_tokens() {
    let (_dir, app, _) = fixture().await;
    copilot_file(&app, "gho-zone-secret");
    let call = "engine('providers.sign_out', provider='github_copilot')";
    let request = denied(&app, &format!("await {call}")).await;
    assert!(!serde_json::to_string(&request)
        .unwrap()
        .contains("gho-zone-secret"));
    let card = engine(request);
    assert_eq!(card.class, Class::Destructive);
    assert_eq!(card.title, "sign out of a provider");
    assert!(card.warning.clone().unwrap().contains("sign in again"));
    let sign_in = item(&card, "sign-in");
    assert_eq!(sign_in.old, Some(json!("connected")));
    assert_eq!(sign_in.new, Some(json!("signed out")));
    assert!(provider_file(&app, "github_copilot").contains("gho-zone-secret"));
    let (_, m) = approve(&app, &format!("(await {call})['data']")).await;
    assert_eq!(result(&m)["signed_out"], true, "{}", result(&m));
    let saved = provider_file(&app, "github_copilot");
    assert!(!saved.contains("gho-zone-secret"), "{saved}");
    let again = result(&run(&app, &format!("(await {call})['text']")).await);
    assert_eq!(again, "nothing to change");
    custom_provider(&app).await;
    assert_eq!(
        refused(
            &app,
            "await engine('providers.sign_out', provider='custom_2')"
        )
        .await,
        "TypeError"
    );
}

#[tokio::test]
async fn providers_sign_out_refuses_when_the_tokens_moved() {
    let (_dir, app, _) = fixture().await;
    copilot_file(&app, "gho-zone-one");
    let app2 = app.clone();
    let error = moved(
        &app,
        "engine('providers.sign_out', provider='github_copilot')",
        || async move { copilot_file(&app2, "gho-zone-two") },
    )
    .await;
    assert!(error.contains("changed since the preview"), "{error}");
    assert!(provider_file(&app, "github_copilot").contains("gho-zone-two"));
}

fn local_mode(dir: &tempfile::TempDir, id: &str, body: &str) -> std::path::PathBuf {
    let modes = project(dir).join(".refact/modes");
    std::fs::create_dir_all(&modes).unwrap();
    let path = modes.join(format!("{id}.yaml"));
    std::fs::write(&path, body).unwrap();
    path
}

async fn resolved_title(app: &AppState, id: &str) -> Option<String> {
    crate::yaml_configs::customization_registry::invalidate_all_registry_caches(app.gcx.clone())
        .await;
    crate::yaml_configs::customization_registry::get_mode_config(app.gcx.clone(), id, None)
        .await
        .map(|m| m.title)
}

#[tokio::test]
async fn modes_delete_override_over_a_builtin_is_a_change_and_the_builtin_returns() {
    let (dir, app, _) = fixture().await;
    let builtin = crate::yaml_configs::project_configs_bootstrap::embedded_defaults("modes")
        .into_iter()
        .find(|(name, _)| name == "explore.yaml")
        .unwrap()
        .1;
    let path = local_mode(
        &dir,
        "explore",
        &builtin.replacen("title: Explore", "title: Zone explore", 1),
    );
    assert_eq!(
        resolved_title(&app, "explore").await.as_deref(),
        Some("Zone explore")
    );
    let call = "engine('modes.delete', kind='modes', mode='explore', scope='local')";
    let card = engine(denied(&app, &format!("await {call}")).await);
    assert!(path.exists(), "a denied delete removed the file");
    assert_eq!(card.class, Class::Change);
    assert_eq!(card.title, "delete a mode");
    assert_eq!(item(&card, "file").new, Some(json!("deleted")));
    assert_eq!(
        item(&card, "falls back to").new,
        Some(json!("builtin modes/explore"))
    );
    assert!(item(&card, "content")
        .old
        .as_ref()
        .unwrap()
        .to_string()
        .contains("Zone explore"));
    let (_, m) = approve(&app, &format!("(await {call})['data']")).await;
    assert_eq!(result(&m)["deleted"], true, "{}", result(&m));
    assert!(!path.exists());
    let title = resolved_title(&app, "explore").await.unwrap();
    assert_ne!(title, "Zone explore");
}

#[tokio::test]
async fn modes_delete_of_a_narrowing_override_is_risky_and_refused_for_this_chats_mode() {
    let (dir, app, _) = fixture().await;
    let path = local_mode(
        &dir,
        "explore",
        "schema_version: 1\nid: explore\ntitle: Zone explore\ntools: [cat]\n",
    );
    let call = "engine('modes.delete', kind='modes', mode='explore', scope='local')";
    session(&app).await.lock().await.thread.mode = "agent".into();
    let card = engine(denied(&app, &format!("await {call}")).await);
    assert_eq!(card.class, Class::Risky);
    assert!(
        card.warning.clone().unwrap().contains("more tools"),
        "{card:?}"
    );
    assert!(path.exists());
    for mode in ["explore", "EXPLORE"] {
        session(&app).await.lock().await.thread.mode = mode.into();
        assert_eq!(
            refused(&app, &format!("await {call}")).await,
            "PermissionError",
            "{mode}"
        );
        assert!(path.exists());
    }
}

#[tokio::test]
async fn modes_delete_sole_definition_is_destructive_with_a_warning() {
    let (dir, app, _) = fixture().await;
    let path = local_mode(
        &dir,
        "zone_only",
        "schema_version: 1\nid: zone_only\ntitle: Zone only\ntools: [cat]\n",
    );
    let call = "engine('modes.delete', kind='modes', mode='zone_only', scope='local')";
    let (request, _) = approve(&app, &format!("await {call}")).await;
    let card = engine(request);
    assert_eq!(card.class, Class::Destructive);
    assert!(card.warning.clone().unwrap().contains("cannot be restored"));
    assert_eq!(item(&card, "falls back to").new, Some(json!("nothing")));
    assert!(!path.exists());
    assert!(resolved_title(&app, "zone_only").await.is_none());
}

#[tokio::test]
async fn modes_delete_refusals_and_moved_file() {
    let (dir, app, _) = fixture().await;
    let path = local_mode(
        &dir,
        "zone_only",
        "schema_version: 1\nid: zone_only\ntitle: Zone only\ntools: [cat]\n",
    );
    assert_eq!(
        refused(
            &app,
            "await engine('modes.delete', kind='modes', mode='zone_only')"
        )
        .await,
        "TypeError"
    );
    assert_eq!(
        refused(
            &app,
            "await engine('modes.delete', kind='modes', mode='agent', scope='local')"
        )
        .await,
        "TypeError"
    );
    session(&app).await.lock().await.thread.mode = "zone_only".into();
    assert_eq!(
        refused(
            &app,
            "await engine('modes.delete', kind='modes', mode='zone_only', scope='local')"
        )
        .await,
        "PermissionError"
    );
    assert!(path.exists());
    session(&app).await.lock().await.thread.mode = "agent".into();
    let moved_path = path.clone();
    let error = moved(
        &app,
        "engine('modes.delete', kind='modes', mode='zone_only', scope='local')",
        || async move {
            std::fs::write(
                &moved_path,
                "schema_version: 1\nid: zone_only\ntitle: Zone moved\ntools: [cat]\n",
            )
            .unwrap();
        },
    )
    .await;
    assert!(error.contains("changed since the preview"), "{error}");
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("Zone moved"));
}

fn shell_policy_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    project(dir).join(".refact/shell_policy.yaml")
}

async fn write_shell_policy(
    app: &AppState,
    edit: impl FnOnce(&mut crate::tools::shell_gate::ShellGatePolicy),
) {
    let mut policy = crate::tools::shell_gate::ShellGatePolicy::default();
    edit(&mut policy);
    crate::tools::shell_gate::save_policy(app.gcx.clone(), &policy)
        .await
        .unwrap();
}

#[tokio::test]
async fn tools_set_shell_policy_classes_and_round_trip() {
    let (dir, app, _) = fixture().await;
    write_shell_policy(&app, |p| p.deny.push("rm -rf /*".into())).await;
    let call = "engine('tools.set_shell_policy', remove_deny=['rm -rf /*'])";
    let card = engine(denied(&app, &format!("await {call}")).await);
    assert_eq!(card.class, Class::Risky);
    assert_eq!(card.title, "change the shell policy");
    assert!(card.warning.clone().unwrap().contains("without asking"));
    assert_eq!(card.scope.as_deref(), Some("this project, all chats"));
    assert_eq!(item(&card, "deny").old, Some(json!("rm -rf /*")));
    assert!(std::fs::read_to_string(shell_policy_file(&dir))
        .unwrap()
        .contains("rm -rf /*"));
    approve(&app, &format!("await {call}")).await;
    let policy = crate::tools::shell_gate::load_policy(app.gcx.clone()).await;
    assert!(
        !policy.deny.contains(&"rm -rf /*".to_string()),
        "{policy:?}"
    );
    let (request, m) = approve(
        &app,
        "(await engine('tools.set_shell_policy', mode='strict', add_deny=['zone-danger']))['data']",
    )
    .await;
    let card = engine(request);
    assert_eq!(card.class, Class::Change, "{card:?}");
    assert!(card.warning.is_none());
    assert!(result(&m)["changed"]["mode"].is_object(), "{}", result(&m));
    let policy = crate::tools::shell_gate::load_policy(app.gcx.clone()).await;
    assert_eq!(policy.mode, crate::tools::shell_gate::ApprovalMode::Strict);
    assert!(policy.deny.contains(&"zone-danger".to_string()));
    for raising in [
        "mode='permissive'",
        "remove_ask=['raw::(){ :|:& };:']",
        "trust_caller_confirmation=False",
        "catalogue=[{'id': 'redirect.device', 'enabled': False}]",
        "catalogue=[{'id': 'redirect.device', 'level': 'low'}]",
    ] {
        let card = engine(
            denied(
                &app,
                &format!("await engine('tools.set_shell_policy', {raising})"),
            )
            .await,
        );
        assert_eq!(card.class, Class::Risky, "{raising}");
        assert!(card.warning.is_some(), "{raising}");
    }
    write_shell_policy(&app, |p| {
        p.trust_caller_confirmation = false;
        p.allow.push("zone-allowed".into());
    })
    .await;
    let card = engine(
        denied(
            &app,
            "await engine('tools.set_shell_policy', trust_caller_confirmation=True, remove_allow=['zone-allowed'], llm_validation={'enabled': True})",
        )
        .await,
    );
    assert_eq!(card.class, Class::Change, "{card:?}");
    write_shell_policy(&app, |p| {
        p.llm_validation.enabled = true;
        p.llm_validation.on_failure = crate::tools::shell_gate::ShellLlmOnFailure::Ask;
    })
    .await;
    let card = engine(
        denied(
            &app,
            "await engine('tools.set_shell_policy', llm_validation={'model': 'zone/other-model'})",
        )
        .await,
    );
    assert_eq!(card.class, Class::Change, "ask_only validator model swap");
    write_shell_policy(&app, |p| {
        p.llm_validation.enabled = true;
        p.llm_validation.authority = crate::tools::shell_gate::ShellLlmAuthority::AskAndAllow;
    })
    .await;
    let card = engine(
        denied(
            &app,
            "await engine('tools.set_shell_policy', llm_validation={'model': 'zone/other-model'})",
        )
        .await,
    );
    assert_eq!(
        card.class,
        Class::Risky,
        "ask_and_allow validator model swap"
    );
    write_shell_policy(&app, |p| {
        p.llm_validation.enabled = true;
        p.llm_validation.on_failure = crate::tools::shell_gate::ShellLlmOnFailure::Ask;
    })
    .await;
    for raising in [
        "llm_validation={'enabled': False}",
        "llm_validation={'authority': 'ask_and_allow'}",
        "llm_validation={'on_failure': 'pass'}",
    ] {
        let card = engine(
            denied(
                &app,
                &format!("await engine('tools.set_shell_policy', {raising})"),
            )
            .await,
        );
        assert_eq!(card.class, Class::Risky, "{raising}");
    }
    let same = result(
        &run(
            &app,
            "(await engine('tools.set_shell_policy', llm_validation={'enabled': True}))['text']",
        )
        .await,
    );
    assert_eq!(same, "nothing to change");
}

#[tokio::test]
async fn tools_set_shell_policy_activating_an_approving_validator_is_risky() {
    use crate::tools::shell_gate::{ShellLlmAuthority, ShellLlmOnFailure};
    let (_dir, app, _) = fixture().await;
    write_shell_policy(&app, |p| {
        p.llm_validation.enabled = false;
        p.llm_validation.authority = ShellLlmAuthority::AskAndAllow;
    })
    .await;
    for raising in [
        "llm_validation={'enabled': True}",
        "llm_validation={'enabled': True, 'model': 'zone/other-model'}",
    ] {
        let card = engine(
            denied(
                &app,
                &format!("await engine('tools.set_shell_policy', {raising})"),
            )
            .await,
        );
        assert_eq!(card.class, Class::Risky, "{raising}");
        assert!(card.warning.is_some(), "{raising}");
    }
    let card = engine(
        denied(
            &app,
            "await engine('tools.set_shell_policy', llm_validation={'model': 'zone/other-model'})",
        )
        .await,
    );
    assert_eq!(card.class, Class::Change, "model of a disabled validator");
    write_shell_policy(&app, |p| {
        p.llm_validation.enabled = true;
        p.llm_validation.on_failure = ShellLlmOnFailure::Pass;
    })
    .await;
    for outside in [0, 121] {
        assert_eq!(
            refused(
                &app,
                &format!(
                    "await engine('tools.set_shell_policy', llm_validation={{'timeout_secs': {outside}}})"
                )
            )
            .await,
            "TypeError",
            "timeout_secs {outside}"
        );
    }
    let card = engine(
        denied(
            &app,
            "await engine('tools.set_shell_policy', llm_validation={'timeout_secs': 1})",
        )
        .await,
    );
    assert_eq!(
        card.class,
        Class::Risky,
        "shorter timeout with on_failure pass"
    );
    assert!(card.warning.is_some());
    let card = engine(
        denied(
            &app,
            "await engine('tools.set_shell_policy', llm_validation={'timeout_secs': 30})",
        )
        .await,
    );
    assert_eq!(card.class, Class::Change, "longer validator timeout");
    write_shell_policy(&app, |p| {
        p.llm_validation.enabled = true;
        p.llm_validation.on_failure = ShellLlmOnFailure::Ask;
    })
    .await;
    let card = engine(
        denied(
            &app,
            "await engine('tools.set_shell_policy', llm_validation={'timeout_secs': 1})",
        )
        .await,
    );
    assert_eq!(
        card.class,
        Class::Change,
        "a timeout asks with on_failure ask"
    );
}

#[tokio::test]
async fn tools_set_shell_policy_refusals_and_moved_file() {
    let (dir, app, _) = fixture().await;
    write_shell_policy(&app, |_| {}).await;
    let before = std::fs::read_to_string(shell_policy_file(&dir)).unwrap();
    for (call, kind) in [
        ("mode='yolo'", "PermissionError"),
        ("add_allow=['*']", "PermissionError"),
        ("add_allow=['.*']", "PermissionError"),
        ("add_allow=['']", "PermissionError"),
        ("add_allow=['re:.*']", "PermissionError"),
        ("add_allow=['raw:*']", "PermissionError"),
        ("add_allow=['re:.']", "PermissionError"),
        ("add_allow=['exec:[!~]*']", "PermissionError"),
        ("add_allow=['re:^[a-m]', 're:^[^a-m]']", "PermissionError"),
        ("add_allow=['ls']", "PermissionError"),
        (
            "add_allow=['git status'], add_deny=['zone-x']",
            "PermissionError",
        ),
        (
            "catalogue=[{'id': 'zone.unknown', 'enabled': False}]",
            "TypeError",
        ),
        ("remove_deny=['zone-not-there']", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('tools.set_shell_policy', {call})")
            )
            .await,
            kind,
            "{call}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(shell_policy_file(&dir)).unwrap(),
        before
    );
    let app2 = app.clone();
    let error = moved(
        &app,
        "engine('tools.set_shell_policy', add_deny=['zone-one'])",
        || async move { write_shell_policy(&app2, |p| p.deny.push("zone-two".into())).await },
    )
    .await;
    assert!(error.contains("changed since the preview"), "{error}");
    let policy = crate::tools::shell_gate::load_policy(app.gcx.clone()).await;
    assert!(!policy.deny.contains(&"zone-one".to_string()));
}

fn hooks_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    project(dir).join(".refact/hooks.yaml")
}

/// The local hooks in the order hooks.get shows them (events by name, file order within one).
async fn local_hooks(dir: &tempfile::TempDir) -> Vec<crate::ext::hooks::HookConfig> {
    let mut hooks = crate::ext::hooks::load_hooks(
        &crate::http::routers::v1::ext_management::make_scope_ext_dirs(
            project(dir).join(".refact"),
            "local",
        ),
    )
    .await;
    hooks.sort_by(|a, b| a.event.as_str().cmp(b.event.as_str()));
    hooks
}

fn write_hooks(dir: &tempfile::TempDir, yaml: &str) {
    std::fs::write(hooks_file(dir), yaml).unwrap();
}

const TWO_HOOKS: &str = "hooks:\n  PreToolUse:\n    - matcher: shell\n      hooks:\n        - type: command\n          command: zone-guard.sh\n  SessionStart:\n    - hooks:\n        - type: command\n          command: zone-start.sh\n";

#[cfg(unix)]
#[tokio::test]
async fn local_writers_refuse_project_files_that_link_into_the_global_config() {
    let (dir, app, _) = fixture().await;
    let refact = project(&dir).join(".refact");
    let config = app.gcx.config_dir.clone();
    std::fs::create_dir_all(config.join("modes")).unwrap();
    std::fs::write(
        config.join("modes/zone_g.yaml"),
        "schema_version: 1\nid: zone_g\ntitle: Zone global\n",
    )
    .unwrap();
    std::fs::write(config.join("hooks.yaml"), "hooks: {}\n").unwrap();
    let _ = std::fs::remove_dir_all(refact.join("modes"));
    std::os::unix::fs::symlink(config.join("modes"), refact.join("modes")).unwrap();
    std::fs::create_dir_all(config.join("integrations.d")).unwrap();
    std::fs::write(
        config.join("integrations.d/cmdline_zdel.yaml"),
        "command: echo zdel\ncommand_workdir: ''\ndescription: zdel\nparameters: []\n",
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(refact.join("integrations.d"));
    std::os::unix::fs::symlink(config.join("integrations.d"), refact.join("integrations.d"))
        .unwrap();
    for (name, target) in [
        ("hooks.yaml", "hooks.yaml"),
        ("shell_policy.yaml", "zone_shell_policy.yaml"),
        ("coordination.yaml", "zone_coordination.yaml"),
        ("indexing.yaml", "zone_indexing.yaml"),
        ("project_information.yaml", "zone_project_information.yaml"),
        ("skills.yaml", "zone_skills.yaml"),
    ] {
        let _ = std::fs::remove_file(refact.join(name));
        std::os::unix::fs::symlink(config.join(target), refact.join(name)).unwrap();
    }
    let targets = [
        "hooks.yaml",
        "zone_shell_policy.yaml",
        "zone_coordination.yaml",
        "zone_indexing.yaml",
        "modes/zone_g.yaml",
        "modes/zone_linked.yaml",
        "integrations.d/cmdline_zone.yaml",
        "integrations.d/cmdline_zdel.yaml",
        "zone_project_information.yaml",
        "zone_skills.yaml",
    ];
    let snapshot = || {
        targets
            .iter()
            .map(|t| std::fs::read(config.join(t)).ok())
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    for call in [
        "engine('hooks.add', scope='local', event='SessionStart', command='zone-evil.sh')",
        "engine('modes.save', kind='modes', id='zone_linked', scope='local', config={'schema_version': 1, 'id': 'zone_linked', 'title': 'Zone'})",
        "engine('modes.delete', kind='modes', mode='zone_g', scope='local')",
        "engine('tools.set_shell_policy', add_deny=['zone-x'])",
        "engine('settings.set', values={'max_resume_burst': 7})",
        "engine('settings.reset', keys=['max_resume_burst'], scope='project')",
        "engine('settings.clear_project_override', keys=['max_resume_burst'])",
        "engine('index.set_settings', scope='project', blocklist=['zone/**'])",
        "engine('integrations.save', name='cmdline_zone', scope='project', settings={'command': 'echo zone', 'command_workdir': '', 'description': 'zone'})",
        "engine('integrations.delete', name='cmdline_zdel', scope='project')",
        "engine('modes.set_project_info', sections={'git_info': {'enabled': False}})",
        "engine('modes.set_skills_trigger', auto_trigger='off')",
    ] {
        assert_eq!(
            refused(&app, &format!("await {call}")).await,
            "PermissionError",
            "{call}"
        );
    }
    assert!(
        snapshot() == before,
        "a project write reached the global config"
    );
}

/// Replaces `.refact/<name>` with a link to `<config>/<target>` holding the same content, so the
/// state the card was computed from does not change.
#[cfg(unix)]
fn link_into_config(dir: &tempfile::TempDir, config: &std::path::Path, name: &str, target: &str) {
    let local = project(dir).join(".refact").join(name);
    let linked = config.join(target);
    std::fs::create_dir_all(linked.parent().unwrap()).unwrap();
    if local.is_dir() {
        std::fs::remove_dir_all(&local).unwrap();
        std::fs::create_dir_all(&linked).unwrap();
    } else if local.exists() {
        std::fs::rename(&local, &linked).unwrap();
    }
    std::os::unix::fs::symlink(&linked, &local).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn local_writers_check_the_project_file_again_before_writing() {
    let (dir, app, _) = fixture().await;
    let config = app.gcx.config_dir.clone();
    for (call, name, target) in [
        (
            "engine('settings.set', values={'max_resume_burst': 7})",
            "coordination.yaml",
            "zone_coordination.yaml",
        ),
        (
            "engine('integrations.save', name='cmdline_zone', scope='project', settings={'command': 'echo zone', 'command_workdir': '', 'description': 'zone'})",
            "integrations.d",
            "integrations.d",
        ),
        (
            "engine('modes.set_project_info', sections={'git_info': {'enabled': False}})",
            "project_information.yaml",
            "zone_project_information.yaml",
        ),
        (
            "engine('modes.set_skills_trigger', auto_trigger='off')",
            "skills.yaml",
            "zone_skills.yaml",
        ),
    ] {
        std::fs::create_dir_all(project(&dir).join(".refact/integrations.d")).unwrap();
        let (task, request) = ask(
            &app,
            &format!("try:\n    r = await {call}\n    out = r['error']\nexcept PermissionError as e:\n    out = str(e)\nout"),
        )
        .await;
        link_into_config(&dir, &config, name, target);
        decide(&app, &request.request_id, Decision::AllowOnce)
            .await
            .unwrap();
        let error = result(&finish(task).await);
        assert!(
            error.as_str().unwrap_or_default().contains("resolves outside this project"),
            "{call}: {error}"
        );
        assert!(
            !config.join("integrations.d/cmdline_zone.yaml").exists(),
            "{call}: wrote a global integration"
        );
    }
}

#[tokio::test]
async fn hooks_add_is_risky_written_and_reports_that_it_will_not_run() {
    let (dir, app, _) = fixture().await;
    let call = "engine('hooks.add', scope='local', event='PreToolUse', matcher='shell', command='zone-check.sh', timeout=20)";
    let card = engine(denied(&app, &format!("await {call}")).await);
    assert!(!hooks_file(&dir).exists(), "a denied add wrote the file");
    assert_eq!(card.class, Class::Risky);
    assert_eq!(card.title, "add a hook");
    assert_eq!(card.scope.as_deref(), Some("this project"));
    assert!(card.warning.as_deref().unwrap().contains("PreToolUse"));
    assert!(card
        .warning
        .as_deref()
        .unwrap()
        .contains("block or rewrite"));
    assert_eq!(item(&card, "command").new, Some(json!("zone-check.sh")));
    assert_eq!(item(&card, "matcher").new, Some(json!("shell")));
    assert!(item(&card, "will run")
        .new
        .as_ref()
        .unwrap()
        .to_string()
        .starts_with("\"no"));
    let (_, m) = approve(&app, &format!("(await {call})['data']")).await;
    let out = result(&m);
    assert_eq!(out["runs"], false, "{out}");
    assert!(out["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("trusted_projects"));
    assert_eq!(out["index"], 0);
    let hooks = local_hooks(&dir).await;
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].command, "zone-check.sh");
    assert_eq!(hooks[0].timeout, Some(20));
    assert_eq!(
        refused(
            &app,
            "await engine('hooks.add', scope='global', event='Stop', command='x')"
        )
        .await,
        "PermissionError"
    );
    assert!(!app.gcx.config_dir.join("hooks.yaml").exists());
    assert_eq!(
        refused(
            &app,
            "await engine('hooks.add', scope='local', event='ZoneEvent', command='x')"
        )
        .await,
        "TypeError"
    );
    let content = std::fs::read_to_string(hooks_file(&dir)).unwrap();
    let error = moved(
        &app,
        "engine('hooks.add', scope='local', event='Stop', command='zone-stop.sh')",
        || async { write_hooks(&dir, TWO_HOOKS) },
    )
    .await;
    assert!(error.contains("changed since the preview"), "{error}");
    assert_ne!(std::fs::read_to_string(hooks_file(&dir)).unwrap(), content);
    assert_eq!(local_hooks(&dir).await.len(), 2);
}

#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn hooks_list_shows_an_untrusted_hook_just_added_with_the_index_remove_expects() {
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let (dir, app, _) = fixture().await;
    write_hooks(&dir, TWO_HOOKS);
    let warm = result(&run(&app, "(await engine('hooks.list'))['data']").await);
    let local_rows = |v: &Value| {
        v["registrations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["scope"] == "local")
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(local_rows(&warm).len(), 2, "{warm}");
    approve(
        &app,
        "await engine('hooks.add', scope='local', event='Stop', command='zone-added.sh')",
    )
    .await;
    let listed = result(&run(&app, "(await engine('hooks.list'))['data']").await);
    let rows = local_rows(&listed);
    assert_eq!(rows.len(), 3, "{listed}");
    let added = rows
        .iter()
        .find(|r| r["command"] == "zone-added.sh")
        .unwrap_or_else(|| panic!("{listed}"));
    assert_eq!(added["runs"], false, "{added}");
    assert!(added["blocked_reason"]
        .as_str()
        .unwrap()
        .contains("trusted_projects"));
    assert_eq!(added["scope"], "local");
    let index = added["index"].as_u64().unwrap();
    let hooks = local_hooks(&dir).await;
    assert_eq!(hooks[index as usize].command, "zone-added.sh");
    for row in &rows {
        let i = row["index"].as_u64().unwrap() as usize;
        assert_eq!(hooks[i].command, row["command"].as_str().unwrap(), "{row}");
    }
    approve(
        &app,
        &format!("await engine('hooks.remove', scope='local', index={index}, expect={{'event': 'Stop', 'command': 'zone-added.sh'}})"),
    )
    .await;
    assert!(local_hooks(&dir)
        .await
        .iter()
        .all(|h| h.command != "zone-added.sh"));
}

#[tokio::test]
async fn hooks_changes_refuse_a_file_with_entries_the_engine_skips() {
    let (dir, app, _) = fixture().await;
    let yaml = "hooks:\n  Stop:\n    - hooks:\n        - type: command\n          command: zone-stop.sh\n        - type: prompt\n          command: zone-prompt\n  ZoneFutureEvent:\n    - hooks:\n        - type: command\n          command: zone-future.sh\n";
    write_hooks(&dir, yaml);
    assert_eq!(
        refused(
            &app,
            "await engine('hooks.add', scope='local', event='Stop', command='zone-new.sh')"
        )
        .await,
        "PermissionError"
    );
    assert_eq!(std::fs::read_to_string(hooks_file(&dir)).unwrap(), yaml);
}

#[tokio::test]
async fn hooks_remove_is_destructive_and_checks_expect() {
    let (dir, app, _) = fixture().await;
    write_hooks(&dir, TWO_HOOKS);
    let index = local_hooks(&dir)
        .await
        .iter()
        .position(|h| h.command == "zone-guard.sh")
        .unwrap();
    let call = format!("engine('hooks.remove', scope='local', index={index}, expect={{'event': 'PreToolUse', 'command': 'zone-guard.sh'}})");
    let card = engine(denied(&app, &format!("await {call}")).await);
    assert_eq!(local_hooks(&dir).await.len(), 2);
    assert_eq!(card.class, Class::Destructive);
    assert_eq!(card.title, "remove a hook");
    let warning = card.warning.clone().unwrap();
    assert!(
        warning.contains("add it back") && warning.contains("blocking tool calls"),
        "{warning}"
    );
    let entry = item(&card, "PreToolUse");
    assert_eq!(entry.old, Some(json!("zone-guard.sh")));
    assert_eq!(entry.new, Some(json!("removed")));
    approve(&app, &format!("await {call}")).await;
    let hooks = local_hooks(&dir).await;
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].command, "zone-start.sh");
    let before = std::fs::read_to_string(hooks_file(&dir)).unwrap();
    for (call, kind) in [
        (
            "scope='local', index=0, expect={'event': 'SessionStart', 'command': 'other.sh'}",
            "TypeError",
        ),
        (
            "scope='local', index=5, expect={'event': 'SessionStart', 'command': 'zone-start.sh'}",
            "TypeError",
        ),
        (
            "scope='global', index=0, expect={'event': 'SessionStart', 'command': 'zone-start.sh'}",
            "PermissionError",
        ),
    ] {
        assert_eq!(
            refused(&app, &format!("await engine('hooks.remove', {call})")).await,
            kind,
            "{call}"
        );
    }
    assert_eq!(std::fs::read_to_string(hooks_file(&dir)).unwrap(), before);
}

#[tokio::test]
async fn hooks_edit_is_risky_and_recomputes_runs() {
    let (dir, app, _) = fixture().await;
    write_hooks(&dir, TWO_HOOKS);
    let index = local_hooks(&dir)
        .await
        .iter()
        .position(|h| h.command == "zone-start.sh")
        .unwrap();
    let call = format!("engine('hooks.edit', scope='local', index={index}, expect={{'event': 'SessionStart', 'command': 'zone-start.sh'}}, command='zone-start-2.sh', timeout=5)");
    let card = engine(denied(&app, &format!("await {call}")).await);
    assert!(local_hooks(&dir)
        .await
        .iter()
        .any(|h| h.command == "zone-start.sh"));
    assert_eq!(card.class, Class::Risky);
    assert_eq!(card.title, "change a hook");
    assert!(card.warning.clone().unwrap().contains("SessionStart"));
    let command = item(&card, "command");
    assert_eq!(command.old, Some(json!("zone-start.sh")));
    assert_eq!(command.new, Some(json!("zone-start-2.sh")));
    assert_eq!(item(&card, "timeout").new, Some(json!(5)));
    let (_, m) = approve(&app, &format!("(await {call})['data']")).await;
    assert_eq!(result(&m)["runs"], false, "{}", result(&m));
    let hooks = local_hooks(&dir).await;
    assert!(hooks
        .iter()
        .any(|h| h.command == "zone-start-2.sh" && h.timeout == Some(5)));
    assert!(hooks.iter().any(|h| h.command == "zone-guard.sh"));
    assert_eq!(
        refused(&app, &format!("await engine('hooks.edit', scope='global', index={index}, expect={{'event': 'SessionStart', 'command': 'zone-start-2.sh'}}, command='x')")).await,
        "PermissionError"
    );
    let same = result(&run(&app, &format!("(await engine('hooks.edit', scope='local', index={index}, expect={{'event': 'SessionStart', 'command': 'zone-start-2.sh'}}, command='zone-start-2.sh'))['text']")).await);
    assert_eq!(same, "nothing to change");
}

const GLOBAL_PRIVACY: &str = "hooks:\n  enabled: true\nprivacy_rules:\n  blocked: ['zone_secret/**', 'zone_other/**']\n  zones:\n    - name: zone_restricted\n      patterns: ['zone_r/**', 'zone_r2/**']\n      send_to: ['zone']\n    - name: normal\n      patterns: ['**']\n      send_to: ['*']\n";

fn global_privacy(app: &AppState) -> std::path::PathBuf {
    app.gcx.config_dir.join("privacy.yaml")
}

async fn saved_global(app: &AppState) -> refact_privacy::PrivacyPolicy {
    crate::privacy::global_only_policy(app.gcx.clone())
        .await
        .unwrap()
}

#[tokio::test]
async fn privacy_relax_unblocks_with_effective_view_and_is_risky() {
    let (dir, app, _) = fixture().await;
    std::fs::write(global_privacy(&app), GLOBAL_PRIVACY).unwrap();
    std::fs::write(
        project(&dir).join(".refact/privacy.yaml"),
        "blocked: ['zone_other/**']\n",
    )
    .unwrap();
    let call = "engine('privacy.relax', unblock=['zone_secret/**', 'zone_other/**'])";
    let card = engine(denied(&app, &format!("await {call}")).await);
    assert!(saved_global(&app)
        .await
        .blocked
        .contains(&"zone_secret/**".to_string()));
    assert_eq!(card.class, Class::Risky);
    assert_eq!(card.title, "relax privacy rules");
    assert!(card
        .warning
        .clone()
        .unwrap()
        .contains("model providers or MCP servers"));
    assert!(card.items.iter().any(|i| i.label == "blocked"
        && i.old == Some(json!("zone_secret/**"))
        && i.new.is_none()));
    let here = item(&card, "zone_other/** here");
    assert_eq!(here.new, Some(json!("still blocked here by project rules")));
    let effective = item(&card, "effective blocked here");
    assert!(effective
        .old
        .as_ref()
        .unwrap()
        .to_string()
        .contains("zone_secret"));
    assert!(!effective
        .new
        .as_ref()
        .unwrap()
        .to_string()
        .contains("zone_secret"));
    assert!(effective
        .new
        .as_ref()
        .unwrap()
        .to_string()
        .contains("zone_other"));
    let (_, m) = approve(&app, &format!("(await {call})['data']")).await;
    assert!(result(&m)["changed"].as_array().is_some(), "{}", result(&m));
    let saved = saved_global(&app).await;
    assert!(saved.blocked.is_empty(), "{saved:?}");
    assert!(std::fs::read_to_string(global_privacy(&app))
        .unwrap()
        .contains("hooks"));
    let live = app.gcx.privacy_policy_load.read().unwrap().policy.clone();
    assert!(live.blocked.contains(&"zone_other/**".to_string()));
    assert!(!live.blocked.contains(&"zone_secret/**".to_string()));
}

#[tokio::test]
async fn privacy_relax_refuses_opening_every_zone_to_every_known_destination() {
    let (_dir, app, _) = fixture().await;
    caps_with(&app, &["zone/model-a"], "zone/emb").await;
    std::fs::write(
        global_privacy(&app),
        "privacy_rules:\n  zones:\n    - name: zone_a\n      patterns: ['a/**']\n      send_to: ['zone_x']\n    - name: zone_b\n      patterns: ['b/**']\n      send_to: ['zone_y']\n    - name: normal\n      patterns: ['**']\n      send_to: ['*']\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(global_privacy(&app)).unwrap();
    let (request, _) = approve(
        &app,
        "await engine('privacy.relax', zone_add_destinations={'zone_a': ['zone_y']})",
    )
    .await;
    assert_eq!(engine(request).class, Class::Risky);
    std::fs::write(global_privacy(&app), &before).unwrap();
    let m = run(
        &app,
        "ids = ['zone']\ntry:\n    await engine('privacy.relax', zone_add_destinations={'zone_a': ids, 'zone_b': ids})\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\n[r, ids]",
    )
    .await;
    let out = result(&m);
    assert!(
        out[0].as_str().unwrap().contains("every known destination"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(global_privacy(&app)).unwrap(),
        before
    );
}

#[tokio::test]
async fn privacy_relax_other_parts_refusals_and_moved_file() {
    let (_dir, app, _) = fixture().await;
    std::fs::write(global_privacy(&app), GLOBAL_PRIVACY).unwrap();
    let (request, _) = approve(&app, "await engine('privacy.relax', zone_remove_patterns={'zone_restricted': ['zone_r2/**']}, zone_add_destinations={'zone_restricted': ['zone2']}, allow_mcp={'zone': ['zone_mcp']})").await;
    let card = engine(request);
    assert_eq!(card.class, Class::Risky);
    assert_eq!(
        item(&card, "zone_restricted.patterns").old,
        Some(json!("zone_r2/**"))
    );
    assert_eq!(
        item(&card, "zone_restricted.send_to").new,
        Some(json!("zone2"))
    );
    let saved = saved_global(&app).await;
    let zone = saved
        .zones
        .iter()
        .find(|z| z.name == "zone_restricted")
        .unwrap();
    assert_eq!(zone.patterns, vec!["zone_r/**".to_string()]);
    assert!(zone.send_to.contains(&"zone2".to_string()));
    let before = std::fs::read_to_string(global_privacy(&app)).unwrap();
    for (call, kind) in [
        (
            "zone_add_destinations={'zone_restricted': ['*']}",
            "PermissionError",
        ),
        ("allow_mcp={'zone': ['*']}", "PermissionError"),
        (
            "zone_remove_patterns={'zone_restricted': ['zone_r/**']}",
            "PermissionError",
        ),
        ("unblock=['zone_missing/**']", "TypeError"),
        ("zone_remove_patterns={'zone_nowhere': ['x']}", "TypeError"),
    ] {
        assert_eq!(
            refused(&app, &format!("await engine('privacy.relax', {call})")).await,
            kind,
            "{call}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(global_privacy(&app)).unwrap(),
        before
    );
    let path = global_privacy(&app);
    let error = moved(
        &app,
        "engine('privacy.relax', unblock=['zone_secret/**'])",
        || async move {
            std::fs::write(
                &path,
                GLOBAL_PRIVACY.replace("'zone_other/**'", "'zone_third/**'"),
            )
            .unwrap();
        },
    )
    .await;
    assert!(error.contains("changed since the preview"), "{error}");
    assert!(saved_global(&app)
        .await
        .blocked
        .contains(&"zone_secret/**".to_string()));
    let only_blocked = "privacy_rules:\n  blocked: ['zone_x/**']\n";
    std::fs::write(global_privacy(&app), only_blocked).unwrap();
    assert_eq!(
        refused(&app, "await engine('privacy.relax', unblock=['zone_x/**'])").await,
        "PermissionError"
    );
    assert_eq!(
        std::fs::read_to_string(global_privacy(&app)).unwrap(),
        only_blocked
    );
}

// ---------------------------------------------------------------------------
// Review findings A2 on the ordinary changes of these areas.

#[tokio::test]
async fn a2_mo_1_override_file_widening_a_base_mode_is_risky_and_refused_for_this_chats_mode() {
    let (dir, app, _) = fixture().await;
    let config = |base: &str| {
        format!("{{'schema_version': 1, 'id': 'zone_ovr', 'base': '{base}', 'match_models': ['*'], 'override': {{'tool_confirm': {{'rules': [{{'match': '*', 'action': 'auto'}}]}}, 'tools_add': ['shell'], 'thread_defaults': {{'auto_approve_dangerous_commands': True}}}}}}")
    };
    session(&app).await.lock().await.thread.mode = "agent".into();
    let own = refused(
        &app,
        &format!(
            "await engine('modes.save', kind='modes', id='zone_ovr', scope='local', config={})",
            config("agent")
        ),
    )
    .await;
    assert_eq!(own, "PermissionError");
    assert!(!project(&dir).join(".refact/modes/zone_ovr.yaml").exists());
    let card = engine(
        denied(
            &app,
            &format!(
                "await engine('modes.save', kind='modes', id='zone_ovr', scope='local', config={})",
                config("explore")
            ),
        )
        .await,
    );
    assert_eq!(card.class, Class::Risky, "{card:?}");
    assert!(card.warning.is_some());
    assert_eq!(item(&card, "widens").new, Some(json!("explore")));
    for key in [
        "{'tools_add': ['shell']}",
        "{'tool_confirm': {'rules': [{'match': 'cat', 'action': 'auto'}]}}",
        "{'thread_defaults': {'auto_approve_editing_tools': True}}",
        "{'allow_mcp': True}",
    ] {
        let card = engine(denied(&app, &format!("await engine('modes.save', kind='modes', id='zone_ovr2', scope='local', config={{'schema_version': 1, 'id': 'zone_ovr2', 'base': 'explore', 'match_models': ['*'], 'override': {key}}})")).await);
        assert_eq!(card.class, Class::Risky, "{key}");
    }
}

#[tokio::test]
async fn a2_mo_2_reordering_confirm_rules_is_widening() {
    let (dir, app, _) = fixture().await;
    local_mode(&dir, "zone_mode", "schema_version: 1\nid: zone_mode\ntitle: Zone\ntools: [cat, shell]\ntool_confirm:\n  rules:\n    - match: shell\n      action: ask\n    - match: '*'\n      action: auto\n");
    let card = engine(denied(&app, "await engine('modes.save', kind='modes', id='zone_mode', config={'tool_confirm': {'rules': [{'match': '*', 'action': 'auto'}, {'match': 'shell', 'action': 'ask'}]}})").await);
    assert_eq!(card.class, Class::Risky);
    let card = engine(denied(&app, "await engine('modes.save', kind='modes', id='zone_mode', config={'tool_confirm': {'rules': [{'match': 'shell', 'action': 'deny'}, {'match': '*', 'action': 'auto'}]}})").await);
    assert_eq!(card.class, Class::Change);
    session(&app).await.lock().await.thread.mode = "zone_mode".into();
    assert_eq!(
        refused(&app, "await engine('modes.save', kind='modes', id='zone_mode', config={'tool_confirm': {'rules': [{'match': '*', 'action': 'auto'}, {'match': 'shell', 'action': 'ask'}]}})").await,
        "PermissionError"
    );
}

#[tokio::test]
async fn a2_mo_new_values_are_shown_in_full() {
    let (dir, app, _) = fixture().await;
    local_mode(
        &dir,
        "zone_mode",
        "schema_version: 1\nid: zone_mode\ntitle: Zone\ntools: [cat]\n",
    );
    let long = "zone-prompt ".repeat(60);
    let card = engine(denied(&app, &format!("await engine('modes.save', kind='modes', id='zone_mode', config={{'prompt': '{long}'}})")).await);
    assert_eq!(item(&card, "prompt").new, Some(json!(long)));
}

#[tokio::test]
async fn a2_mo_3_pinned_model_passes_the_egress_check() {
    let (dir, app, _) = fixture().await;
    local_mode(
        &dir,
        "zone_mode",
        "schema_version: 1\nid: zone_mode\ntitle: Zone\ntools: [cat]\n",
    );
    std::fs::write(
        app.gcx.config_dir.join("privacy.yaml"),
        "privacy_rules:\n  zones:\n    - name: zone_guarded\n      patterns: ['**/zone_guarded/**']\n      send_to: ['model']\n",
    )
    .unwrap();
    let guarded = project(&dir).join("zone_guarded");
    std::fs::create_dir_all(&guarded).unwrap();
    std::fs::write(guarded.join("a.txt"), "zone").unwrap();
    let path = json!(refact_python_kernel::hostfs::virtual_path(
        &guarded.join("a.txt"),
        if cfg!(windows) {
            refact_python_kernel::hostfs::Platform::Windows
        } else {
            refact_python_kernel::hostfs::Platform::Unix
        },
    ));
    let task = spawn(&app, &format!("open({path}).read()\nawait engine('modes.save', kind='modes', id='zone_mode', config={{'title': 'Zone two'}})"));
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    let m = run(&app, &format!("open({path}).read()\nawait engine('modes.save', kind='modes', id='zone_mode', config={{'model_defaults': {{'default': {{'model': 'zoneother/m'}}}}}})")).await;
    assert_eq!(error_type(&m), "PermissionError", "{:?}", m.extra);
}

#[tokio::test]
async fn a2_pv_1_tighten_cannot_relax_through_zone_order() {
    let (_dir, app, _) = fixture().await;
    std::fs::write(
        global_privacy(&app),
        "privacy_rules:\n  zones:\n    - name: zone_public\n      patterns: ['README.md']\n      send_to: ['*']\n    - name: zone_secret\n      patterns: ['src/**']\n      send_to: []\n    - name: normal\n      patterns: ['**']\n      send_to: ['zone']\n",
    )
    .unwrap();
    let before = std::fs::read_to_string(global_privacy(&app)).unwrap();
    assert_eq!(
        refused(
            &app,
            "await engine('privacy.tighten', zone_add_patterns={'zone_public': ['src/**']})"
        )
        .await,
        "PermissionError"
    );
    assert_eq!(refused(&app, "await engine('privacy.tighten', new_zone={'name': 'zone_new', 'patterns': ['lib/**'], 'send_to': ['zone2']})").await, "PermissionError");
    assert_eq!(
        std::fs::read_to_string(global_privacy(&app)).unwrap(),
        before
    );
    let (request, _) = approve(&app, "await engine('privacy.tighten', zone_add_patterns={'zone_secret': ['lib/**']}, new_zone={'name': 'zone_new', 'patterns': ['vendor/**'], 'send_to': []})").await;
    assert_eq!(engine(request).class, Class::Change);
    let saved = saved_global(&app).await;
    assert_eq!(saved.zones[0].name, "zone_new", "{saved:?}");
    assert!(saved
        .zones
        .iter()
        .any(|z| z.name == "zone_secret" && z.patterns.contains(&"lib/**".to_string())));
}

#[tokio::test]
async fn zones_with_secret_like_names_can_be_tightened_and_relaxed() {
    let (_dir, app, _) = fixture().await;
    std::fs::write(
        global_privacy(&app),
        "privacy_rules:\n  zones:\n    - name: secret\n      patterns: ['a/**']\n      send_to: []\n    - name: zone_secret\n      patterns: ['b/**']\n      send_to: []\n    - name: credentials\n      patterns: ['c/**']\n      send_to: ['zone']\n    - name: normal\n      patterns: ['**']\n      send_to: ['*']\n",
    )
    .unwrap();
    let (request, _) = approve(&app, "await engine('privacy.tighten', zone_add_patterns={'secret': ['a2/**'], 'zone_secret': ['b2/**'], 'credentials': ['c2/**']}, zone_remove_destinations={'credentials': ['zone']})").await;
    let card = engine(request);
    assert_eq!(card.class, Class::Change);
    assert_eq!(item(&card, "secret.patterns").new, Some(json!("a2/**")));
    assert_eq!(item(&card, "credentials.send_to").old, Some(json!("zone")));
    let saved = saved_global(&app).await;
    for (zone, pattern) in [
        ("secret", "a2/**"),
        ("zone_secret", "b2/**"),
        ("credentials", "c2/**"),
    ] {
        assert!(
            saved
                .zones
                .iter()
                .any(|z| z.name == zone && z.patterns.contains(&pattern.to_string())),
            "{saved:?}"
        );
    }
    let (request, _) = approve(
        &app,
        "await engine('privacy.relax', zone_add_destinations={'credentials': ['zone2']})",
    )
    .await;
    assert_eq!(
        item(&engine(request), "credentials.send_to").new,
        Some(json!("zone2"))
    );
    let m = run(&app, "try:\n    await engine('privacy.relax', zone_add_destinations={'secret': ['https://u:p@zone.example']})\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\nr").await;
    let text = result(&m).as_str().unwrap().to_string();
    assert!(text.contains("looks like a secret"), "{text}");
}

async fn caps_with(app: &AppState, models: &[&str], embedding: &str) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    for id in models {
        caps.chat_models.insert(
            id.to_string(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: id.to_string(),
                    n_ctx: 64_000,
                    enabled: true,
                    ..Default::default()
                },
                supports_tools: true,
                ..Default::default()
            }),
        );
    }
    caps.defaults.chat_default_model = models[0].into();
    caps.embedding_model.base.id = embedding.into();
    let mut state = app.gcx.caps_state.write().await;
    state.caps = Some(Arc::new(caps));
    state.last_attempted_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
}

#[tokio::test]
async fn a2_md_1_and_2_provider_moves_from_unset_slots_and_other_models_are_risky() {
    let (_dir, app, _) = fixture().await;
    caps_with(&app, &["zone/model-a", "zoneother/model-b"], "zone/emb").await;
    let card = engine(denied(&app, "await engine('models.set_defaults', scope='project', slots={'chat': {'model': 'zoneother/model-b'}})").await);
    assert_eq!(card.class, Class::Risky, "{card:?}");
    let card = engine(denied(&app, "await engine('models.set_defaults', scope='project', slots={'chat': {'model': 'zone/model-a', 'temperature': 0.5}})").await);
    assert_eq!(card.class, Class::Change, "{card:?}");
    let card = engine(denied(&app, "await engine('models.set_defaults', scope='global', slots={'embedding_model': 'zoneother/emb'})").await);
    assert_eq!(card.class, Class::Risky, "{card:?}");
    let card = engine(denied(&app, "await engine('models.set_defaults', scope='global', slots={'completion_model': 'zoneother/comp'})").await);
    assert_eq!(card.class, Class::Risky, "{card:?}");
}

#[tokio::test]
async fn a2_pr_1_unknown_model_id_is_refused() {
    let (_dir, app, _) = fixture().await;
    create_provider(
        &app,
        "openai_2",
        json!({"base_provider":"openai","api_key":"sk-zone","enabled":true}),
    )
    .await;
    assert_eq!(
        refused(&app, "await engine('providers.set_model_enabled', name='openai_2', model='zone-typo-model', enabled=True)").await,
        "TypeError"
    );
}

#[tokio::test]
async fn a2_to_1_unreadable_tool_config_is_a_failure() {
    let (_dir, app, _) = fixture().await;
    std::fs::write(
        app.gcx.config_dir.join("builtin_tools.yaml"),
        "tools: [unclosed\n",
    )
    .unwrap();
    let m = run(
        &app,
        "await engine('tools.set_enabled', tool='cat', enabled=True)",
    )
    .await;
    assert_eq!(error_type(&m), "RuntimeError", "{:?}", m.extra);
    let message = m.extra["python"]["error"]["message"].to_string();
    assert!(message.contains("cannot be read"), "{message}");
}

/// Adds a custom model through the route core, from another thread, so the registry moves.
fn moved_model(gcx: &Arc<crate::global_context::GlobalContext>, id: &str) {
    let (gcx, id) = (gcx.clone(), id.to_string());
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                crate::providers::http::add_provider_custom_model(
                    gcx,
                    "openai_2",
                    &id,
                    serde_json::from_value(json!({"n_ctx": 4096})).unwrap(),
                )
                .await
                .unwrap();
            })
    })
    .join()
    .unwrap();
}

/// A2-TE-1: for each ordinary change of these areas, Deny leaves the fingerprint unchanged and a
/// state moved between the card and the answer is refused without applying.
#[tokio::test]
async fn a2_te_1_deny_and_moved_state_for_ordinary_changes() {
    let (dir, app, _) = fixture().await;
    caps_with(&app, &["openai_2/zone-model", "zone/model-a"], "zone/emb").await;
    create_provider(&app, "openai_2", json!({"base_provider":"openai","api_key":"sk-zone","enabled":true,"custom_models":{"zone-model":{"n_ctx":8192}}})).await;
    local_mode(
        &dir,
        "zone_mode",
        "schema_version: 1\nid: zone_mode\ntitle: Zone\ntools: [cat]\n",
    );
    std::fs::write(app.gcx.config_dir.join("builtin_tools.yaml"), "tools: {}\n").unwrap();
    std::fs::write(global_privacy(&app), GLOBAL_PRIVACY).unwrap();
    crate::yaml_configs::project_information::load_project_information_config(app.gcx.clone())
        .await;
    let other = Arc::new(tokio::sync::Mutex::new(
        crate::chat::types::ChatSession::new("zone-refresh".into()),
    ));
    app.chat
        .sessions
        .write()
        .await
        .insert("zone-refresh".into(), other.clone());
    let files = {
        let (dir, app) = (project(&dir), app.gcx.config_dir.clone());
        move || -> String {
            let mut all = String::new();
            for path in [
                dir.join(".refact/model_defaults.yaml"),
                dir.join(".refact/modes/zone_mode.yaml"),
                dir.join(".refact/project_information.yaml"),
                dir.join(".refact/skills.yaml"),
                app.join("providers.d/openai_2.yaml"),
                app.join("builtin_tools.yaml"),
                app.join("privacy.yaml"),
            ] {
                all.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
                all.push('\u{0}');
            }
            all
        }
    };
    let calls: Vec<(&str, Box<dyn Fn() + Send>)> = vec![
        ("engine('models.set_defaults', scope='project', slots={'chat': {'temperature': 0.3}})", {
            let p = project(&dir).join(".refact/model_defaults.yaml");
            Box::new(move || std::fs::write(&p, "chat:\n  model: zone/model-a\n").unwrap())
        }),
        ("engine('providers.set_model_enabled', name='openai_2', model='zone-model', enabled=True)", {
            let gcx = app.gcx.clone();
            Box::new(move || moved_model(&gcx, "zone-moved-1"))
        }),
        ("engine('providers.add_custom_model', name='openai_2', id='zone-two', config={'n_ctx': 4096})", {
            let gcx = app.gcx.clone();
            Box::new(move || moved_model(&gcx, "zone-moved-2"))
        }),
        ("engine('providers.remove_custom_model', name='openai_2', model='zone-model')", {
            let gcx = app.gcx.clone();
            Box::new(move || moved_model(&gcx, "zone-moved-3"))
        }),
        ("engine('modes.save', kind='modes', id='zone_mode', config={'title': 'Zone two'})", {
            let p = project(&dir).join(".refact/modes/zone_mode.yaml");
            Box::new(move || std::fs::write(&p, "schema_version: 1\nid: zone_mode\ntitle: Moved\ntools: [cat]\n").unwrap())
        }),
        ("engine('modes.set_project_info', sections={'git_info': {'enabled': False}})", {
            let p = project(&dir).join(".refact/project_information.yaml");
            Box::new(move || std::fs::write(&p, "enabled: false\n").unwrap())
        }),
        ("engine('modes.set_skills_trigger', auto_trigger='off')", {
            let p = project(&dir).join(".refact/skills.yaml");
            Box::new(move || std::fs::write(&p, "auto_trigger: inject_full\n").unwrap())
        }),
        ("engine('tools.set_enabled', tool='cat', enabled=False)", {
            let p = app.gcx.config_dir.join("builtin_tools.yaml");
            Box::new(move || std::fs::write(&p, "tools:\n  cat:\n    enabled: false\n").unwrap())
        }),
        ("engine('privacy.tighten', block=['zone_more/**'])", {
            let p = global_privacy(&app);
            Box::new(move || std::fs::write(&p, GLOBAL_PRIVACY.replace("'zone_other/**'", "'zone_moved/**'")).unwrap())
        }),
    ];
    for (call, mutate) in calls {
        let before = files();
        denied(&app, &format!("await {call}")).await;
        assert_eq!(files(), before, "Deny changed state: {call}");
        let error = moved(&app, call, || async { mutate() }).await;
        assert!(
            error.contains("changed since the preview"),
            "{call}: {error}"
        );
    }
    let before = other.lock().await.trajectory_version;
    denied(
        &app,
        "await engine('tools.refresh', chat_id='zone-refresh')",
    )
    .await;
    assert_eq!(other.lock().await.trajectory_version, before);
    let other2 = other.clone();
    let error = moved(
        &app,
        "engine('tools.refresh', chat_id='zone-refresh')",
        || async move {
            other2.lock().await.trajectory_version += 1;
        },
    )
    .await;
    assert!(error.contains("changed since the preview"), "{error}");
}
