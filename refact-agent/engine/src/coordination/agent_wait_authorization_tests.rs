use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::coordination::deadlock_identity_tests::{
    app_with_workspace, create_running_record, parked_run, parked_ticket, work_scope,
};
use crate::tools::tool_background_agents::execute_agents_wait;
use refact_work_core::{AdmissionRequest, CoordinationSettings, WorkKey};
use tokio::sync::Mutex;

async fn run_unauthorized_wait(in_any: bool) -> (String, String, bool) {
    let (mut app, _temp) = app_with_workspace().await;
    app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
        CoordinationSettings {
            max_active_engine: 4,
            max_active_project: 4,
            max_active_root: 4,
            ..CoordinationSettings::default()
        },
    ));
    let caller = WorkKey::trajectory("wait-root").run(1);
    let _caller_ticket = app
        .work_scheduler
        .reserve(AdmissionRequest::new(caller, work_scope()))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let own_id = create_running_record(&app, "wait-root", "subchat-own", None).await;
    let foreign_id = create_running_record(&app, "foreign-root", "subchat-foreign", None).await;
    let _own_ticket = parked_ticket(&app, parked_run(&own_id)).await;
    let _foreign_ticket = parked_ticket(&app, parked_run(&foreign_id)).await;
    let ccx = Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            Vec::new(),
            "wait-root".to_string(),
            None,
            "openai/gpt-4o".to_string(),
            None,
            None,
        )
        .await,
    ));
    let args = if in_any {
        HashMap::from([
            (
                "agent_ids".to_string(),
                serde_json::json!([own_id, foreign_id]),
            ),
            ("timeout_ms".to_string(), serde_json::json!(1000)),
        ])
    } else {
        HashMap::from([
            ("agent_ids".to_string(), serde_json::json!([foreign_id])),
            ("timeout_ms".to_string(), serde_json::json!(1000)),
        ])
    };
    let caller_chat_id = "wait-root".to_string();
    let completed = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let saw_caller_edge = Arc::new(AtomicBool::new(false));
    let probe = {
        let app = app.clone();
        let completed = completed.clone();
        let ready = ready.clone();
        let saw_caller_edge = saw_caller_edge.clone();
        std::thread::spawn(move || {
            ready.store(true, Ordering::SeqCst);
            while !completed.load(Ordering::SeqCst) {
                if app
                    .wait_graph
                    .edges()
                    .iter()
                    .any(|edge| edge.from.work.id == caller_chat_id)
                {
                    saw_caller_edge.store(true, Ordering::SeqCst);
                }
                std::thread::yield_now();
            }
        })
    };
    while !ready.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    let error = execute_agents_wait(ccx, &"unauthorized-wait".to_string(), &args)
        .await
        .unwrap_err();
    completed.store(true, Ordering::SeqCst);
    probe.join().unwrap();
    (error, foreign_id, saw_caller_edge.load(Ordering::SeqCst))
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn single_target_wait_with_unauthorized_id_declares_no_edge_in_flight() {
    let (error, foreign_id, saw_caller_edge) = run_unauthorized_wait(false).await;
    assert_eq!(
        error,
        format!("Background agent '{foreign_id}' not found in this chat.")
    );
    assert!(
        !error.starts_with(refact_work_core::DEADLOCK_CODE),
        "{error}"
    );
    assert!(
        !saw_caller_edge,
        "unauthorized wait briefly declared an edge"
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn await_any_with_unauthorized_id_declares_no_edge_in_flight() {
    let (error, _, saw_caller_edge) = run_unauthorized_wait(true).await;
    assert!(error.contains("not found in this chat"), "{error}");
    assert!(
        !error.starts_with(refact_work_core::DEADLOCK_CODE),
        "{error}"
    );
    assert!(
        !saw_caller_edge,
        "unauthorized wait briefly declared an edge"
    );
}
