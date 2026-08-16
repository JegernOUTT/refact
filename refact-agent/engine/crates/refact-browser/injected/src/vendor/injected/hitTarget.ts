import { trimStringWithEllipsis } from '../isomorphic/stringUtils';
import type { RefactBuiltins } from './utilityScript';

type HitTargetBuiltins = RefactBuiltins & Readonly<{
  getComputedStyle: Window['getComputedStyle'];
  documentElementsFromPoint: (root: Document, x: number, y: number) => Element[];
  documentElementFromPoint: (root: Document, x: number, y: number) => Element | null;
  shadowElementsFromPoint: (root: ShadowRoot, x: number, y: number) => Element[];
  shadowElementFromPoint: (root: ShadowRoot, x: number, y: number) => Element | null;
  addWindowEventListener: (type: string, listener: EventListener, options: AddEventListenerOptions) => void;
  removeWindowEventListener: (type: string, listener: EventListener, options: EventListenerOptions) => void;
  preventDefault: (event: Event) => void;
  stopPropagation: (event: Event) => void;
  stopImmediatePropagation: (event: Event) => void;
}>;

export type HitTargetAction = 'hover' | 'tap' | 'mouse' | 'drag';

export type HitTargetPoint = Readonly<{
  x: number;
  y: number;
}>;

export type HitTargetResult =
  | Readonly<{ status: 'done' }>
  | Readonly<{ status: 'intercepted'; description: string }>
  | Readonly<{ status: 'not_connected' }>
  | Readonly<{ status: 'skipped' }>;

type HitTargetCheckResult = 'done' | Readonly<{ hitTargetDescription: string }>;

type Interceptor = {
  listener: EventListener;
  events: Set<string>;
  blockAllEvents: boolean;
  result: HitTargetCheckResult | undefined;
};

const autoClosingTags = [
  'AREA',
  'BASE',
  'BR',
  'COL',
  'COMMAND',
  'EMBED',
  'HR',
  'IMG',
  'INPUT',
  'KEYGEN',
  'LINK',
  'MENUITEM',
  'META',
  'PARAM',
  'SOURCE',
  'TRACK',
  'WBR',
];
const booleanAttributes = ['checked', 'selected', 'disabled', 'readonly', 'multiple'];

export class HitTargetController {
  private readonly global: typeof globalThis;
  private readonly builtins: HitTargetBuiltins;
  private readonly interceptors: Map<number, Interceptor>;
  private nextInterceptorId = 1;

  constructor(global: typeof globalThis, builtins: RefactBuiltins) {
    this.global = global;
    this.builtins = builtins as HitTargetBuiltins;
    this.interceptors = new builtins.Map<number, Interceptor>();
  }

  expectHitTarget(hitPoint: HitTargetPoint, targetElement: Element): HitTargetResult {
    if (!targetElement?.isConnected)
      return { status: 'not_connected' };
    return this.toResult(this.checkHitTarget(hitPoint, targetElement));
  }

  install(
    targetElement: Element,
    action: HitTargetAction,
    hitPoint: HitTargetPoint | undefined,
    blockAllEvents = false,
  ): Readonly<{ status: 'installed'; id: number }> | HitTargetResult {
    if (!targetElement?.isConnected)
      return { status: 'not_connected' };
    if (hitPoint) {
      const preliminaryResult = this.checkHitTarget(hitPoint, targetElement);
      if (preliminaryResult !== 'done')
        return this.toResult(preliminaryResult);
    }
    if (action === 'drag')
      return { status: 'skipped' };

    const events = new this.builtins.Set<string>(this.eventNames(action));
    const id = this.nextInterceptorId++;
    const interceptor: Interceptor = {
      events,
      blockAllEvents,
      result: undefined,
      listener: (() => {}) as EventListener,
    };
    const listener: EventListener = event => {
      if (!events.has(event.type) || !event.isTrusted)
        return;
      const point = this.eventPoint(event);
      if (interceptor.result === undefined && point)
        interceptor.result = this.checkHitTarget(point, targetElement);
      if (interceptor.blockAllEvents || (interceptor.result !== 'done' && interceptor.result !== undefined)) {
        this.builtins.preventDefault(event);
        this.builtins.stopPropagation(event);
        this.builtins.stopImmediatePropagation(event);
      }
    };
    interceptor.listener = listener;
    for (const eventName of events)
      this.builtins.addWindowEventListener(eventName, listener, { capture: true, passive: false });
    this.interceptors.set(id, interceptor);
    return { status: 'installed', id };
  }

  take(id: number): HitTargetResult {
    const interceptor = this.interceptors.get(id);
    if (!interceptor)
      throw new Error(`Unknown hit-target interceptor ${id}`);
    this.interceptors.delete(id);
    for (const eventName of interceptor.events)
      this.builtins.removeWindowEventListener(eventName, interceptor.listener, { capture: true });
    return this.toResult(interceptor.result ?? 'done');
  }

  private checkHitTarget(hitPoint: HitTargetPoint, targetElement: Element): HitTargetCheckResult {
    const roots: (Document | ShadowRoot)[] = [];
    let parentElement = targetElement;
    while (parentElement) {
      const root = enclosingShadowRootOrDocument(parentElement);
      if (!root)
        break;
      roots.push(root);
      if (root.nodeType === 9)
        break;
      parentElement = root.host;
    }

    let hitElement: Element | undefined;
    for (let index = roots.length - 1; index >= 0; index--) {
      const root = roots[index];
      const elements = this.builtins.arrayFrom(
        root.nodeType === 9
          ? this.builtins.documentElementsFromPoint(root as Document, hitPoint.x, hitPoint.y)
          : this.builtins.shadowElementsFromPoint(root as ShadowRoot, hitPoint.x, hitPoint.y),
      );
      const singleElement = root.nodeType === 9
        ? this.builtins.documentElementFromPoint(root as Document, hitPoint.x, hitPoint.y)
        : this.builtins.shadowElementFromPoint(root as ShadowRoot, hitPoint.x, hitPoint.y);
      if (singleElement && elements[0] && parentElementOrShadowHost(singleElement) === elements[0]) {
        const style = this.builtins.getComputedStyle(singleElement);
        if (style?.display === 'contents')
          elements.unshift(singleElement);
      }
      if (elements[0] && elements[0].shadowRoot === root && elements[1] === singleElement)
        elements.shift();
      const innerElement = elements[0];
      if (!innerElement)
        break;
      hitElement = innerElement;
      if (index && innerElement !== (roots[index - 1] as ShadowRoot).host)
        break;
    }

    const hitParents: Element[] = [];
    while (hitElement && hitElement !== targetElement) {
      hitParents.push(hitElement);
      hitElement = hitElement.assignedSlot ?? parentElementOrShadowHost(hitElement);
    }
    if (hitElement === targetElement)
      return 'done';

    const hitTargetDescription = previewNode(hitParents[0] || this.global.document.documentElement);
    let rootHitTargetDescription: string | undefined;
    let element: Element | undefined = targetElement;
    while (element) {
      const index = hitParents.indexOf(element);
      if (index !== -1) {
        if (index > 1)
          rootHitTargetDescription = previewNode(hitParents[index - 1]);
        break;
      }
      element = parentElementOrShadowHost(element);
    }
    if (rootHitTargetDescription) {
      return {
        hitTargetDescription: `${hitTargetDescription} from ${rootHitTargetDescription} subtree intercepts pointer events`,
      };
    }
    return { hitTargetDescription: `${hitTargetDescription} intercepts pointer events` };
  }

  private eventNames(action: Exclude<HitTargetAction, 'drag'>): readonly string[] {
    if (action === 'hover')
      return ['mousemove'];
    if (action === 'tap')
      return ['pointerdown', 'pointerup', 'pointercancel', 'touchstart', 'touchend', 'touchcancel'];
    return [
      'mousedown',
      'mouseup',
      'pointerdown',
      'pointerup',
      'click',
      'auxclick',
      'dblclick',
      'contextmenu',
    ];
  }

  private eventPoint(event: Event): HitTargetPoint | undefined {
    if ('touches' in event) {
      const touch = (event as TouchEvent).touches[0];
      return touch ? { x: touch.clientX, y: touch.clientY } : undefined;
    }
    if ('clientX' in event && 'clientY' in event) {
      const pointer = event as MouseEvent | PointerEvent;
      return { x: pointer.clientX, y: pointer.clientY };
    }
    return undefined;
  }

  private toResult(result: HitTargetCheckResult): HitTargetResult {
    if (result === 'done')
      return { status: 'done' };
    return { status: 'intercepted', description: result.hitTargetDescription };
  }
}

function parentElementOrShadowHost(element: Element): Element | undefined {
  if (element.parentElement)
    return element.parentElement;
  if (!element.parentNode)
    return undefined;
  if (element.parentNode.nodeType === 11 && (element.parentNode as ShadowRoot).host)
    return (element.parentNode as ShadowRoot).host;
  return undefined;
}

function enclosingShadowRootOrDocument(element: Element): Document | ShadowRoot | undefined {
  let node: Node = element;
  while (node.parentNode)
    node = node.parentNode;
  if (node.nodeType === 11 || node.nodeType === 9)
    return node as Document | ShadowRoot;
  return undefined;
}

function previewNode(node: Node): string {
  if (node.nodeType === 3)
    return oneLine(`#text=${node.nodeValue || ''}`);
  if (node.nodeType !== 1)
    return oneLine(`<${node.nodeName.toLowerCase()} />`);
  const element = node as Element;
  const attributes: string[] = [];
  for (let index = 0; index < element.attributes.length; index++) {
    const { name, value } = element.attributes[index];
    if (name === 'style')
      continue;
    attributes.push(!value && booleanAttributes.includes(name) ? ` ${name}` : ` ${name}="${value}"`);
  }
  attributes.sort((left, right) => left.length - right.length);
  const attributeText = trimStringWithEllipsis(attributes.join(''), 500);
  if (autoClosingTags.includes(element.nodeName))
    return oneLine(`<${element.nodeName.toLowerCase()}${attributeText}/>`);
  const children = element.childNodes;
  let onlyText = children.length <= 5;
  for (let index = 0; index < children.length; index++)
    onlyText = onlyText && children[index].nodeType === 3;
  const text = onlyText ? element.textContent || '' : children.length ? '…' : '';
  return oneLine(
    `<${element.nodeName.toLowerCase()}${attributeText}>${trimStringWithEllipsis(text, 50)}</${element.nodeName.toLowerCase()}>`,
  );
}

function oneLine(value: string): string {
  return value.replace(/\n/g, '↵').replace(/\t/g, '⇆');
}
