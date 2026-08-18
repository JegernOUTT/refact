# chrome tool tutorial

## overview

Text-first batched browser automation. You read pages as text, not as pictures.

The loop is: navigate -> read the returned snapshot refs -> act by ref -> repeat. Any batch that
changes the page attaches a ref-annotated ARIA snapshot under `page.snapshot` automatically, so you
do NOT need an `accessibility_snapshot` step after navigating. Screenshots are opt-in and cost far
more than the text tree.

Each `[ref=eN]` handle in that snapshot is an element address: act with `locator.by=ref`. Refs come
from the most recent snapshot. Use an explicit `accessibility_snapshot` step only to re-read a page
that did NOT change, or to scope to a subtree with `locator`/`depth`.

Canonical batch:

```json
{"steps":[
  {"action":"navigate","url":"https://example.com"},
  {"action":"click","locator":{"by":"ref","value":"e5"}},
  {"action":"fill","locator":{"by":"ref","value":"e7"},"text":"hi"}
]}
```

Pass this object as `request`; e5/e7 stand for handles minted by the snapshot the previous batch
returned. ONE call can carry many steps, unlike one-action-per-call servers.

Page report: a page-changing batch returns `page` with the final URL and title, `page.status` when
the main document answered with a non-2xx status, `page.console` error/warning COUNTS (full text
stays in `console` and `tab_log`), and `page.snapshot`. Snapshots inline their YAML when small; a
large tree is written to a `text/yaml` artifact and `page.snapshot` carries the head plus
`{artifact:{kind,mime,path,bytes}}`, `lines`, `bytes`, and `truncated:true`. Locator-driven actions
echo a canonical Playwright-style locator in `locator_echo` so a run stays auditable after the refs
expire.

`page_context` picks the page-changed context: `snapshot` (the default) attaches the ref-annotated
ARIA snapshot and NO image, `screenshot` attaches a policy-sized image instead, `both` attaches
each, `none` attaches only the page header. The snapshot is attached only when the batch actually
changed the page.

`attach_screenshot` is the tri-state screenshot override and wins over `page_context`: true = always
attach, false = never attach, omitted = follow `page_context`. An explicit `screenshot` step still
returns its own image even when false, and still adds the report screenshot under the default
`snapshot` mode.

`network` controls per-request report volume: `summary` (the default) emits one
`method url status bytes ms` line per request, `full` keeps request and response headers, `none`
drops per-request entries. Route interception telemetry and the detail returned by
wait_for_request and wait_for_response stay visible in every mode.

Batch-level `block_service_workers` bypasses service workers so route interception sees every
request.

`help` is this tutorial: `{"action":"help"}` returns the topic index plus this overview, and
`{"action":"help","topic":"<name>"}` returns one section. A batch containing only `help` steps is
answered from documentation and never launches the browser.

## locators

Ref-first element address or composable fallback locator.

Fallback vocabulary: `ref`; `role` with name/description, exact or regex, and
checked/pressed/selected/expanded/disabled/level/include_hidden filters; `test_id` with configurable
`attribute` (defaults to data-testid); `text`, `label`, `placeholder`, `alt_text`, `title`, `css`,
`xpath`, `id`, `name`, and `autocomplete`.

Compose with zero-based `nth` (-1 is last), `first`/`last`, `locator` (nested, evaluated under each
outer match), `filter` (has/has_not/has_text/has_not_text/visible), `and`/`or`, or an outermost-first
`frames` chain.

`exact` is a case-sensitive whole-string match; a regex ignores it. Regex options are
`{source, flags}` objects: `regex`, `name_regex`, and `description_regex`.

Non-selecting actions are strict: ambiguous locators fail loudly with the match count.
`within` is a deprecated CSS scope kept for compatibility; use `locator` for chaining.

## navigation

navigate, reload, go_back, go_forward.

`set_content` replaces the whole document with raw html, so fixtures need no server or file; it waits
for `wait_until` (domcontentloaded, load, or networkidle; load by default) and re-bootstraps refs
like a navigation, so take a fresh accessibility_snapshot afterwards.

`page_content` returns the full serialized document including its doctype, inline under 8KB and
otherwise as an artifact path.

`add_script_tag` and `add_style_tag` inject into the current document and take exactly one of `url`
or `content`, plus an optional `script_type` such as module; both wait for a url to finish loading
and fail the step if it errors.

`add_init_script` evaluates `content` before any page script on every later navigation in this
session and mints the id it returns, so never send it an id; `remove_init_script` takes that id, and
`reset` drops every init script.

## input

click, click_if_exists, hover, focus, blur, scroll_to, press_key, drag_and_drop, drop_files.

`drag_and_drop` accepts source/target locators or refs plus optional `source_position` and
`target_position`.

Click, hover, fill, clear, check, and uncheck auto-wait for actionability.

Coordinate mouse escape hatch: mouse_move, mouse_down, mouse_up, mouse_click_xy, mouse_drag_xy, and
mouse_wheel use main-frame viewport CSS pixels and bypass locator resolution. Use these only for
canvas, map, and vision-driven UIs with no addressable element; locator/ref actions remain the
default. Locator handlers and overlay auto-dismiss do NOT guard `mouse_*` coordinate actions: an
overlay that would be dismissed before a locator action will still swallow a coordinate click.

Touch and low-level keyboard: `tap` takes either a locator (full actionability and hit-target checks,
like click) or x/y coordinates, and requires touch emulation from an earlier set_viewport step with
`has_touch` true. `insert_text` types into the focused element with one input event and no key
events, which suits IME-style entry but skips keyboard shortcuts; it focuses an optional locator
first. `press_sequentially` focuses its locator and then sends real per-character key events with an
optional `delay_ms` (default 0) for inputs driven by keystroke handlers such as autocomplete; prefer
`fill` for ordinary form entry.

`dispatch_event` sends a synthetic DOM event to a locator regardless of visibility, inferring the
event class from `event_type` (click gives MouseEvent, keydown KeyboardEvent, dragstart DragEvent,
and so on); `event_init` supplies the initialisation properties, with bubbles, cancelable, and
composed defaulting to true.

## forms

fill, clear, select_option, check, uncheck.

`fill` takes `clear_first` (defaults true) and `verify` (defaults true); `clear` takes `verify`.

## waits

`wait_for_function` is the way to wait on arbitrary app state: it evaluates `expression` until the
result is truthy, defaults to 100/250/500/1000ms poll intervals unless `polling_ms` fixes one, and
with a `locator` re-resolves the element each retry and passes it as the first argument, so a
re-rendered node is tolerated. A thrown expression fails immediately instead of retrying.

wait_for_popup, wait_for_selector, wait_for_navigation, wait_for_url, wait_for_text,
wait_for_network_idle, wait_for_load_state, wait_for_element_hidden, wait_for_element_stable.

Put `wait_for_popup` immediately before the popup-producing click in ONE batch; the returned popup
becomes active for later steps.

`wait_for_url` reads a plain-string `pattern` as a case-sensitive substring of the current URL, and a
`{source, flags}` pattern as a regex just like wait_for_request. Plain text is never globbed here,
unlike the glob `pattern` used by route and wait_for_request.

`wait_for_selector` takes an optional `state`: attached (the default, any match in the DOM), visible
(a match with a non-empty box), hidden (no visible match, including no match at all), or detached
(no match). Every state stays non-strict, so several matches never fail the step.

`wait_for_console_message` is the active console wait: it blocks until a console entry matches
optional `contains` and `level` (log, warning, or error, where error also covers uncaught page
errors), returns that entry redacted, and sees messages produced earlier in the same batch.
`tab_log` stays the passive read of buffered output.

Never use `wait_seconds` for readiness; use `wait_for_response`, `wait_for_load_state`, or
`wait_for_selector` for genuine synchronization.

## network

`wait_for_request` and `wait_for_response` accept a URL string or `{source,flags}` regex; both also
take an optional `method` filter and wait_for_response an optional `status`, so a wait can skip an
early 404 and land on the following 200 for the same pattern. Completed requests also appear in the
report.

`http_request` sends an HTTP call that shares the page's cookie jar in both directions: matching
cookies for the target domain and path are attached, and response Set-Cookie headers are written back
into the browser, so a logged-in page and the API call see the same session. Send `url` plus optional
`method`, `headers`, and exactly one of `body`, `body_json` (auto application/json), or `form` (auto
urlencoded); http and https only. Results carry status, final URL after redirects,
content-type/content-length (set `full_headers=true` for every header), and the body inline when it
stays under 8KB, otherwise an artifact path. Cookie values are never inlined, only the count and
names. Set `fail_on_status=true` to fail the step on a non-2xx status.

`set_network_conditions` takes latency_ms, download_kbps, upload_kbps, an optional offline flag, and
an optional `preset` of slow-3g, fast-3g, or slow-4g using Chrome DevTools values; explicit
parameters override the preset and omitted bandwidth stays unlimited. `set_cpu_throttling` takes
`rate`, a slowdown multiplier where 1 is off. `reset` clears both.

## routes

route/unroute/list_routes control HTTP interception.

`route` registers a persistent `{pattern,handler}` with fulfill, abort, continue, fallback, or
fetch_and_fulfill; `unroute` removes one pattern or all routes; `list_routes` returns active routes
in evaluation order with `order` and `times_remaining`.

Several routes may share a pattern: the newest matching route runs first, a `fallback` handler hands
the request to the next older matching route, then to the HAR replay, then to the network. Optional
`times` on a route expires it after that many matches, including matches consumed by a traversed
fallback.

`fulfill` takes `body`, or `path` to serve a file (relative paths stay inside the runtime artifact
directory and may not escape it, content type inferred from the extension unless `content_type` is
set), or `json` for a JSON body; status defaults to 200.

`fetch_and_fulfill` performs the real request from the engine (up to 20 redirects, forwarding the
page's own request headers) and fulfills with the real response, optionally overriding `status`,
`response_headers`, and `body`.

Cookie, Host, and Content-Length request headers keep their original values on continue and
fetch_and_fulfill.

Text route bodies are UTF-8 and encoded to base64 on the CDP wire; set `body_base64=true` when `body`
already contains base64 binary data.

URL patterns are globs (`*`, `**`, `{a,b}`) or `{source,flags}` regexes; `?` is literal and
JavaScript route predicates are not supported. Page-level routes may not observe requests served by
a service worker.

`abort` reasons: failed, aborted, timedout, accessdenied, connectionclosed, connectionreset,
connectionrefused, connectionaborted, connectionfailed, namenotresolved, internetdisconnected,
addressunreachable, blockedbyclient, blockedbyresponse.

## websockets

`route_web_socket` and `unroute_web_socket` install page-level WebSocket routing.
`send_web_socket_message` supplies mock page messages and `wait_for_web_socket_frame` waits for
observed traffic.

WebSocket routes take `mode`: "mock" (default) answers the page entirely from send_web_socket_message
and never reaches the real server, while "intercept" connects to the real server and relays both
directions.

Per-direction `on_page_message` and `on_server_message` take "forward" (default, relay and report),
"capture" (report and satisfy wait_for_web_socket_frame but do NOT relay), or "drop" (block the
frame; it is reported as dropped and never satisfies wait_for_web_socket_frame).

`close_web_socket` simulates a server-side close with an optional `code` and `reason` delivered to
the page's onclose. Frame reports carry the page-requested subprotocols per socket, and frame
payloads are redacted.

## har

`start_har_recording` and `stop_har_recording` write a runtime-owned HAR artifact.
`start_har_recording` `update` names an existing HAR to record into, replacing matched method+url
entries and appending new ones. `route_from_har` replays it with `not_found` abort or fallback for
misses. HAR output is returned as a path and summary, never inlined.

`mode` is full or minimal; `content` is omit, embed, or attach; `url_filter` narrows what is
recorded or replayed.

## tabs

open_tab, close_tab, switch_tab, list_tabs.

`open_tab` accepts optional `device`/`url`; `close_tab` accepts an optional `tab` and otherwise
closes active. Closing active selects the preceding tab in adoption order, the next tab when closing
the first, or leaves no active tab.

`target` on the request and `tab` on a step take `{"type":"active"}` or `{"type":"id","id":"..."}`.

## frames

Locators take an outermost-first `frames` chain. Each owner must resolve to exactly one iframe or
frame element. Same-process frames are supported; out-of-process frames fail explicitly.

## dialogs

`handle_dialog` arms the next dialog with `accept` and optional `prompt_text`; unarmed dialogs
auto-dismiss except beforeunload, which is accepted.

## uploads

`set_input_files` sets files on a file input directly. `expect_file_chooser` arms the chooser for a
flow that opens one. `drop_files` delivers files to a drop target as if dragged from outside the
page.

## downloads

`wait_for_download` waits for a download and takes an optional `save_as`; `cancel_download` cancels
one by `id` or the latest. Failed downloads report `failure_reason`.

## emulation

set_viewport, emulate_media, set_locale, set_timezone, set_user_agent, set_geolocation, set_offline,
and set_extra_http_headers persist across adopted tabs and popups.

Window vs viewport: `set_viewport` is device-metrics emulation (it changes what the page measures,
not the window on screen); `set_window_bounds` moves and resizes the actual OS window with
x/y/width/height, any subset. `set_window_bounds` needs a headed browser: in headless there is no OS
window, so it succeeds without applying and tells you to use set_viewport. `reset` does not touch
window bounds.

`emulate_media` takes color_scheme (light, dark, no-preference), reduced_motion (reduce,
no-preference), forced_colors (active, none), contrast (more, less, custom, no-preference), and
`media`.

Cookie state uses get_cookies, set_cookies, clear_cookies. Web storage uses get_storage, set_storage,
clear_storage with `kind` local or session.

`storage_state` and `set_storage_state` use Playwright's
`{cookies,origins:[{origin,local_storage}]}` login-reuse shape, and `indexed_db` true additionally
snapshots or restores IndexedDB best-effort: every database and object store of the current origin,
capped at 200 records per store with `truncated` flagged, values must survive JSON round-trip so
Blob, File, and ArrayBuffer entries are lost, and restore recreates each named database from
scratch.

`grant_permissions` state granted, denied, or prompt and `clear_permissions` control origin
permissions. `set_http_credentials` shares the lazy Fetch path with routing. Cookie, storage, and
credential values are redacted in reports.

## devices

`emulate_device` applies one named Playwright device (viewport, DPR, mobile, touch, and user agent
together). `list_devices` returns the 200+ names with an optional `filter`. mobile, tablet, and
desktop stay as aliases accepted by both emulate_device and open_tab. An unknown name is a hard error
listing the closest matches.

## clock

`clock_install` pins fake time (optional `time` as unix ms or ISO string, current time by default)
and must run before the page caches Date.

`clock_fast_forward` jumps ahead firing each due timer AT MOST ONCE while `clock_run_for` advances
firing ALL callbacks along the way, so a 60s interval fires once under fast_forward and 60 times
under run_for.

`clock_pause_at` stops time at an instant, `clock_resume` restarts it, `clock_set_fixed_time` freezes
Date.now while leaving timers running, and `clock_set_system_time` shifts time silently without
firing timers.

`ticks` takes milliseconds or "MM:SS"/"HH:MM:SS"; the clock is session-scoped across tabs and
navigations until `reset` clears it.

## screenshots

Screenshots support full_page, clip, type, quality, scale, omit_background, animations, caret, mask,
mask_color, and style. `screenshot_element` uses locator or ref.

`screenshot_elements` takes `locators` plus `compose` (grid composes one labeled contact sheet,
separate returns one image per locator).

`capture_element_states` captures one locator across `states` (default, hover, focus, active) as a
labeled strip.

`pdf` supports Chromium print options (landscape, print_background, scale, format, width, height,
margins, page_ranges, prefer_css_page_size, tagged, outline) and returns an artifact path.

## artifacts

PDF, coverage, HAR, large snapshots, oversized CDP results, and oversized http_request bodies are
written to the runtime artifact directory and returned as
`{artifact:{kind,mime,path,bytes}}` plus a summary rather than inlined.

Relative `path` values on route fulfill resolve inside that artifact directory and may not escape
it.

## screencast

`capture_frames` records a burst and returns ONE composed filmstrip image (up to a 4x6 grid, each
cell labelled +NNNms) plus per-frame artifact paths and the percentage of pixels that changed against
the previous frame, so animations and transient UI are readable even without looking at pixels. It
takes `duration_ms` (defaults to 1000, capped at 10000) with either `frame_count` (2-24, defaults to
8) or `interval_ms`, and scopes to an element with `locator` or to the whole document with
`full_page`. Out-of-range values are hard errors.

`screencast_start` and `screencast_stop` bracket a manual session that auto-stops at 30000ms or 60
frames and reports that cap as a warning; `screencast_stop` composes a filmstrip unless
`compose=false`. The filmstrip is always attached, even when attach_screenshot is false.

## assertions

`expect` retries with a 5000ms default and supports state, text/value, attribute/class/CSS/id/
property, role/accessibility, count, URL/title, and ARIA snapshot matchers. Assertion failures report
expected and last received values; set `soft=true` to record a failure and continue the batch.

Set `not=true` to invert any matcher: the step retries until the matcher stops matching and a timeout
reports the still-matching value.

`to_have_text` and `to_contain_text` also accept an array of expectations across every match, exact
and same-length for to_have_text, an ordered subset for to_contain_text. `to_have_attribute` without
`expected` asserts presence only, `to_have_css` takes an optional `pseudo` of before or after,
`to_be_checked` takes `checked` or `indeterminate` (never both), and `to_be_in_viewport` takes an
optional `ratio` between 0 and 1.

`expect_poll` evaluates `expression` and retries until the value satisfies `matcher` (equals,
contains, gt, lt, matches_regex) against `expected`, reporting attempts and elapsed like expect; it
also honours `soft`.

## readouts

Never fake these with eval or expect.

`bounding_box` returns viewport CSS-pixel x/y/width/height or null when the element is not visible.
`count` returns the match count without strictness. `input_value` returns the live value property of
an input, textarea, or select and fails on any other element. `all_texts` returns the text of every
match with `mode` inner_text or text_content plus an optional `limit`, reporting the true total.
`element_state` returns visible, enabled, editable, checked, and stable in one read.

Other inspection: get_text, get_html, get_attribute, extract_links, extract_table, dom_snapshot,
accessibility_snapshot, styles, tab_log.

`accessibility_snapshot` takes `mode` (ai or default, defaulting to ai), `refs` (mint refs; defaults
to true in ai mode), `boxes`, `locator` to scope to a subtree, `depth` to limit nesting (deeper
children collapse into a truncated-count marker), and `max_chars`.

Instrumentation: `start_coverage` and `stop_coverage` opt into precise JavaScript and CSS usage
tracking and return bounded per-URL summaries plus a full JSON artifact.

## handlers

`add_locator_handler` and `remove_locator_handler` register recurring interstitial dismissal.
Locator handlers use `{type:"click"}` or `{type:"steps",steps:[...]}`; handler steps are ordinary
browser steps minus the ones that manage session-wide plumbing (route, unroute, list_routes, reset,
http_request, add_init_script, remove_init_script, cdp_send, and the clock_* family).

`times` bounds how often a handler fires and `no_wait_after` skips waiting for it to become hidden.

`dismiss_overlays` is the one-shot version. It only clicks known cookie/consent/close buttons, so a
legitimate modal (possibly holding your target) is never deleted; the default handler that runs
before pointer actions behaves the same way. Pass `aggressive: true` to also delete large fixed
overlays with a z-index above 1000 — an explicit, destructive opt-in that never runs automatically.
Other advanced steps: eval, highlight_element, highlight
(locator/ref plus optional `style` and `label`), hide_highlight, annotate (locator/ref plus `text`),
and fixed-delay wait_seconds.

Handlers and overlay auto-dismiss do NOT guard `mouse_*` coordinate actions.

## cdp

`cdp_send` is the raw Chrome DevTools Protocol escape hatch for the long tail that has no dedicated
step: send `method` plus optional `params`, with `target` "page" (default, the active tab) or
"browser".

Prefer a dedicated step whenever one exists, because those carry actionability, redaction, and reset
bookkeeping that raw CDP does not. State set through cdp_send is invisible to list_routes and reset,
so undo it yourself; Emulation and Network mutations come back with a warning saying exactly that.

`Browser.close` is refused, and so is `Target.closeTarget` aimed at the tab this session drives.
Results return inline as JSON under 8KB and as an artifact path beyond it, cookie and storage values
are redacted, and CDP errors surface verbatim on one bounded line.

## authenticators

`add_virtual_authenticator` enables passkey testing and mints the authenticator id it returns, so
never send it an id. `remove_virtual_authenticator`, `list_credentials`, `add_credential`,
`clear_credentials`, and `set_user_verified` address that returned id. Credential ids, private keys,
user handles, blobs, and user names are redacted from reports.

## reset

`reset` is the escape hatch for sticky plumbing: one call drops every network route, HAR replay,
WebSocket route, locator handler, init script, virtual authenticator, and the fake clock, turns
offline off, drops network and CPU throttling, and clears media, viewport, device, geolocation, and
permission overrides, reporting what it cleared with counts.

It leaves cookies, storage, open tabs, and the current page untouched, and it does not touch window
bounds.

## troubleshooting

Stale refs: refs come from the most recent snapshot. If a batch changed the page, use the refs from
that batch's `page.snapshot`, not from an older one. Re-read with `accessibility_snapshot` when in
doubt.

Strict-mode failure: a non-selecting action found several matches. Add `nth`, `first`/`last`, or a
`filter` to disambiguate; `count` and `all_texts` are non-strict and safe for probing.

Flaky waits: replace `wait_seconds` with `wait_for_selector`, `wait_for_load_state`,
`wait_for_response`, or `wait_for_function`.

A coordinate click did nothing: overlays are not auto-dismissed for `mouse_*`. Dismiss the overlay
first, or use a locator action.

Routes not firing: page-level routes may not observe requests served by a service worker; set
batch-level `block_service_workers`. Check `list_routes` for evaluation order and
`times_remaining`.

Sticky state leaking between scenarios: call `reset`.
