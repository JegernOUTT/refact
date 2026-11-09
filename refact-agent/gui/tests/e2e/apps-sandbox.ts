import {
  buildAppDocument,
  APP_IFRAME_SANDBOX,
} from "../../src/features/Apps/appDocument";
import {
  AppBridgeHost,
  createAppChannel,
  type AppAppearance,
} from "../../src/features/Apps/appSdk";
import { requestAppCapture } from "../../src/features/Apps/appCapture";
import type { AppCaptureParams } from "../../src/features/Apps/appSdk";

const APP_HTML = `<!doctype html><html><head><style>
body{margin:0;font:14px sans-serif;background:#f4f4f4;color:#111}
html[data-refact-appearance=dark] body{background:#202020;color:#eee}
#scroller{position:absolute;left:10px;top:10px;width:120px;height:100px;overflow:auto;border:1px solid #000}
.row{height:50px}
#canvas{position:absolute;left:150px;top:10px}
#gl{position:absolute;left:260px;top:10px;width:40px;height:40px}
#form{position:absolute;left:10px;top:130px;width:300px}
#pw{position:absolute;left:10px;top:250px;width:120px;height:24px;background:#ff00ff}
#secret{position:absolute;left:150px;top:250px;width:100px;height:24px;background:#ff00ff}
</style></head><body>
<div id="scroller"><div class="row" style="background:#ff0000"></div><div class="row" style="background:#00ff00"></div><div class="row" style="background:#0000ff"></div><div class="row" style="background:#ffff00"></div></div>
<canvas id="canvas" width="100" height="60"></canvas>
<canvas id="gl" width="40" height="40"></canvas>
<div id="form"><input id="text" value="initial"> <input id="check" type="checkbox"> <select id="sel"><option value="a">Alpha</option><option value="b">Bravo</option></select><br><textarea id="area" rows="2">x</textarea></div>
<input id="pw" type="password" value="hunter2">
<div id="secret" data-sensitive>TOPSECRET</div>
<div id="log"></div>
<script>
const c=document.getElementById('canvas').getContext('2d');c.fillStyle='#00ffff';c.fillRect(0,0,100,60);
document.getElementById('gl').getContext('webgl');
window.__events=[];
refactApp.on('theme',p=>__events.push(['theme',p]));
refactApp.on('data:feed',p=>__events.push(['data',p]));
refactApp.on('state',p=>__events.push(['state',p]));
window.__fetchResult='pending';
fetch('https://example.com/x').then(()=>{__fetchResult='allowed'},e=>{__fetchResult='blocked:'+e.name});
window.__evalResult=(()=>{try{eval('1');return 'allowed'}catch(e){return 'blocked:'+e.name}})();
window.__parentAccess=(()=>{try{return String(window.parent.document.title)}catch(e){return 'blocked:'+e.name}})();
window.__forgedId=null;
</script></body></html>`;

type Rejection = { reason: string; data: unknown };
const host = document.getElementById("host") as HTMLDivElement;
let iframe: HTMLIFrameElement | null = null;
let bridge: AppBridgeHost | null = null;
let generation = 0;
const rejections: Rejection[] = [];
const calls: { method: string; params: unknown }[] = [];
let appState: unknown = { count: 1 };

function mount(appearance: AppAppearance = "light"): Promise<void> {
  bridge?.detach();
  iframe?.remove();
  generation += 1;
  const channel = createAppChannel();
  const frame = document.createElement("iframe");
  frame.setAttribute("sandbox", APP_IFRAME_SANDBOX);
  frame.setAttribute("referrerpolicy", "no-referrer");
  frame.title = "app";
  frame.style.cssText = "width:400px;height:300px;border:0;display:block";
  frame.srcdoc = buildAppDocument({
    html: APP_HTML,
    channel,
    generation,
    appearance,
    initialState: appState,
  });
  iframe = frame;
  let onReady: () => void = () => undefined;
  const ready = new Promise<void>((r) => (onReady = r));
  const b = new AppBridgeHost({
    channel,
    generation,
    getFrame: () => iframe,
    onRejected: (reason, data) => rejections.push({ reason, data }),
    handlers: {
      ready: () => {
        setTimeout(onReady, 0);
        return { appearance, state: appState };
      },
      "state.get": () => ({ state: appState }),
      "state.set": (p) => {
        appState = p.state;
        calls.push({ method: "state.set", params: p });
        return { ok: true };
      },
      "event.emit": (p) => {
        calls.push({ method: "event.emit", params: p });
        return null;
      },
      "context.stage": (p) => {
        calls.push({ method: "context.stage", params: p });
        return { staged: true };
      },
    },
  });
  bridge = b;
  b.attach(window);
  host.appendChild(frame);
  return ready;
}

const api = {
  mount,
  get generation() {
    return generation;
  },
  get channel() {
    return bridge?.channel ?? null;
  },
  rejections,
  calls,
  notify: (method: "theme" | "state" | "data", params: never) =>
    bridge?.notify(method, params),
  capture: async (params: AppCaptureParams = {}) => {
    if (!bridge) throw new Error("no bridge");
    return requestAppCapture(bridge, params);
  },
  rawCapture: (params: AppCaptureParams = {}) =>
    bridge?.request("capture", params),
  postFromHost: (data: unknown) => window.postMessage(data, "*"),
  samplePng: async (dataUrl: string, points: [number, number][]) => {
    const img = new Image();
    img.src = dataUrl;
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    const ctx = c.getContext("2d");
    if (!ctx) throw new Error("no ctx");
    ctx.drawImage(img, 0, 0);
    return {
      width: img.naturalWidth,
      height: img.naturalHeight,
      pixels: points.map(([x, y]) =>
        Array.from(ctx.getImageData(x, y, 1, 1).data),
      ),
    };
  },
};
(window as unknown as { __apps: typeof api }).__apps = api;
void mount().then(() => document.body.setAttribute("data-ready", "1"));
