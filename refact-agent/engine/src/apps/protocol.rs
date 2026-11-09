use serde_json::{json, Value};

pub fn authoring_contract() -> Value {
    json!({
        "protocol_version": 1,
        "sdk_version": "1.0.0",
        "authoring": {
            "workflow": [
                "Use App tools only; no browser, external docs, filesystem writes or dev server are required to author a plain HTML app.",
                "app create {slug,name} returns a stable app_id and draft revision. Read app.json and ui/index.html with app read_file. Resource paths are relative inside the app, never absolute or parent traversal.",
                "Preserve schema_version=1 and app_id in app.json. Write each resource with app write_file and the latest returned string expected_revision; each write changes the draft revision. Reread on conflicts, never guess revisions.",
                "app_source configure/update/remove edits the draft manifest only. app_build publish {app_id,expected_revision} explicitly validates and publishes an immutable bundle; it does not run build scripts. No shell is needed for plain HTML/CSS/JS. Optional builds require normal shell/process approvals.",
                "app_binding list returns the numeric workspace revision. Attach with that expected_revision to create a chat binding, then app_view open with app_id and binding_id. Wait for app_view list to return a real view; requested is not registered or focused.",
                "After publish/rollback old viewers are invalidated. Select a current view matching app_id, binding_id and published revision before app_screenshot capture {view_id}. No frontend means no screenshot; SVG fallback is not PNG. Inspect source status and runtime diagnostics with app_inspect get."
            ],
            "icons":"Use icon: lucide:globe or another supported Lucide identifier, or write a small bundled SVG with App write_file and set icon to its relative resource path (for example ui/icon.svg). SVG must have a positive viewBox and contain only svg/g/path/line/polyline/polygon/rect/circle/ellipse geometry with bounded numbers and fill/stroke currentColor, none or hex colors. Maximum 16 KiB, 64 elements and depth 8. No scripts, event handlers, style/CSS, transforms, text, images, use/references, animation, URLs, entities, DTD or XML declarations. Unsupported artwork falls back to a generic app icon; no remote icon URLs.",
            "change_approval":"An app is pinned to this chat when it already has a binding here; a visible pane is not required. Model-requested changes to other apps require exact human approval, including attaching an unbound app. App tools remain available in all user-facing modes. Source grants and privacy checks remain separate authority requirements.",
            "named_actions": {
                "event":{"action_id":"notify","channel":"weather","operation":{"type":"event_emit"},"permissions":[]},
                "state":{"action_id":"save","channel":"state","operation":{"type":"state_set"},"permissions":[]},
                "refresh":{"action_id":"refresh","channel":"weather","operation":{"type":"source_refresh","source_id":"weather"},"permissions":[]},
                "payloads":"event_emit accepts bounded JSON; state_set payload is exactly {expected_revision:<numeric current state revision>,value:<new whole state>}; source_refresh takes {} or null and waits for a real new read of an already granted active HTTP/interval source. Prefer setState for ordinary view state saves because the host handles its CAS revision. Action permissions must be empty; no arbitrary execution is available."
            },
            "manifest_example": {
                "schema_version":1,"app_id":"<UUID returned by create>","name":"Weather",
                "entrypoint":"ui/index.html","sources":[{
                    "source_id":"weather","channel":"weather",
                    "driver":{"type":"http_poll","url":"https://example.com/weather.json","every_ms":60000},
                    "permissions":[{"type":"network","origin":"https://example.com"}]
                }],"actions":[],"permissions":[]
            }
        },
        "sdk": {
            "global":"window.refactApp (injected before app scripts; do not import or replace)",
            "ready":"await refactApp.ready() resolves {appearance,state,...}. The SDK begins the handshake automatically, but await it before initializing UI or calling host methods. It is cached, not a DOM ready event.",
            "methods":{
                "getState()":"Promise of the JSON state value, not {state}; host selects this view's binding or standalone scope.",
                "setState(value)":"Promise of host response; replaces the whole JSON value. Host manages numeric CAS revisions. Handle rejection; do not fabricate revisions or tokens.",
                "emit(name,payload)":"Promise of acknowledgement; emits an event, not an executable command or grant.",
                "stageContext(text,label?)":"Stages a user-reviewable context item. It does not silently send a chat message.",
                "requestAction(actionId,payload)":"Requests a declared bounded named operation. Supported operations are event_emit, state_set and source_refresh; never arbitrary tool/shell execution. Declaration is not authorization; errors must be surfaced.",
                "on(name,handler)":"Registers a listener and returns an unsubscribe function; off(name,handler) also removes it."
            },
            "events": {
                "state":"on('state', ({state}) => render(state)): payload is an object with state, not the bare state value.",
                "theme":"on('theme', ({appearance}) => ...); appearance is light or dark. CSS tokens update automatically.",
                "data":"on('data', ({name,payload}) => ...): all channel notifications.",
                "data:<channel>":"on('data:weather', envelope => ...): managed sources deliver an envelope, NOT raw response JSON. Read envelope.data only when envelope.status === 'data'; surface envelope.status otherwise (error details may be in envelope.data). Tool/app event emissions may supply their own payload shape; validate before use."
            },
            "source_envelope": {
                "app_id":"uuid","source_id":"weather",
                "generation":"source generation","seq":"monotonic source sequence",
                "channel":"weather","status":"data",
                "data":{"temperature":21}
            },
            "example":"const app = window.refactApp; function render(state) { document.querySelector('#value').textContent = String(state?.count ?? 0); } app.on('state', ({state}) => render(state)); app.on('data:weather', envelope => { if (envelope.status === 'data') document.querySelector('#weather').textContent = JSON.stringify(envelope.data); }); app.ready().then(({state}) => render(state)).catch(error => { document.querySelector('#value').textContent = error.message; });",
            "transport":"Use SDK methods, not hand-built postMessage. Wire requests are {type:'refact-app',v:1,channel,generation,kind:'request',id,method,params}; methods are ready/state.get/state.set/event.emit/context.stage/action.request. Host validates frame identity, channel, generation and authority. Host view tokens are not model-visible."
        },
        "state_controls": {
            "drafts":"Keep unsaved input drafts separate from persisted state. An incoming state event must not overwrite a dirty draft simply because focus moved to another control. Snapshot the draft before awaiting a save, serialize UI writes, and clear dirty ownership only after an explicit successful save of that exact draft version.",
            "initialization":"Do not enable save/change controls before ready and initial getState resolve. Loading existing state must not write defaults back over a persisted binding. Show Saved only after setState resolves; handle CAS failure and preserve the user draft.",
            "action_fields":"ActionDefinition accepts action_id, channel, operation, description and permissions. Use description for the human label, not an undeclared name field. Source polling grant/start is only needed after final publication. Inspect runtime data after attaching a binding; app get/build status inspect unbound draft metadata."
        },
        "sources": {
            "managed":"Browser fetch, XMLHttpRequest, EventSource and WebSocket are blocked by CSP. Configure sources in app.json/app_source, publish, obtain grants, then control start; subscribe to data:<channel> in the UI. Never work around CSP with remote scripts, proxies in the iframe, or fabricated data presented as live.",
            "drivers":{
                "manual":{"type":"manual"},"interval":{"type":"interval","every_ms":1000},
                "http_poll":{"type":"http_poll","url":"https://example.com/data.json","every_ms":1000},
                "sse":{"type":"sse","url":"https://example.com/events"},
                "websocket":{"type":"websocket","url":"wss://example.com/events"},
                "process":{"type":"process","process_id":"<managed process id>","chat_id":"<optional owning chat>"},
                "internal":{"type":"internal","topic":"<supported topic>"}
            },
            "grants":"permissions are declarations, not grants. app_source control {operation:'grant'|'revoke'|'start'|'stop'|'pause'|'resume',app_id,source_id,expected_revision} uses the published string revision and normal approval. For network sources app_source grant_endpoint binds the exact origin plus explicit allowed_private_ips to the published revision with user approval; local/private endpoints require that capability. No confirmed/approved flags, arbitrary filesystem paths or cross-chat process authority. Republish requires grants for the new revision.",
            "diagnosis":"app_source status reports supervised source state. Loading, permission-required and error states are real UI states; do not render success before a data envelope arrives."
        },
        "csp":{
            "policy":"default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; form-action 'none'",
            "assets":"Use inline scripts/styles or bundle-relative script src, stylesheet href and image assets. The host inlines supported published bundle assets; do not rely on CDN imports, module dependency loading or remote URLs. Iframe sandbox has scripts only: no same-origin, host DOM, storage authority or external fetch. Do not navigate the app document or create nested frames."
        },
        "theme":{
            "behavior":"Host supplies live --rf-* CSS variables, inherited pane typography and zero-specificity element defaults. Authored CSS can override them; avoid hardcoded colors and overriding host variables. documentElement data-refact-appearance and refactApp.appearance track light/dark.",
            "layout":"The iframe is the pane viewport. Body has no implicit padding or forced overflow clamp. Use class refact-app-content on a document body/wrapper for standard content padding, optionally --rf-app-content-padding. Full-height apps should own their inner padding and use shrinking flex/grid tracks with min-height:0; do not put a 100vh panel beneath a header. Give each intended content region one scroll owner. Native controls and scrollbars receive tokenized zero-specificity defaults that authored CSS may override.",
            "tokens":["--rf-bg","--rf-color-fg","--rf-color-muted","--rf-surface-1","--rf-surface-2","--rf-border","--rf-color-accent","--rf-color-danger","--rf-color-success","--rf-focus-ring","--rf-radius-card","--rf-space-3","--rf-font-ui","--rf-font-mono","--rf-app-font-family","--rf-app-font-size","--rf-app-line-height"],
            "example":"html,body{min-height:100%;margin:0} body{background:var(--rf-bg);color:var(--rf-color-fg)} .card{background:var(--rf-surface-1);border:1px solid var(--rf-border);border-radius:var(--rf-radius-card);padding:var(--rf-space-3)}"
        }
    })
}

pub fn diagnostics(record: Value, sources: Value, views: Value, events: Value) -> Value {
    json!({"app":record,"sources":sources,"views":views,"events":events,
        "diagnostics_version":1,
        "limitations":["No app code executed by inspection", "Viewer presence does not prove SDK readiness", "No arbitrary action adapter is implied"]})
}
