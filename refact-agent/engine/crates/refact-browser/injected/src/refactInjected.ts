/**
 * Copyright 2026 SmallCloud Technologies Ltd.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import {
  type CheckedState,
  getAriaDisabled,
  getCheckedState,
  getReadonly,
  isElementVisible,
} from './vendor/injected/domUtils';
import {
  HitTargetController,
  type HitTargetAction,
  type HitTargetPoint,
  type HitTargetResult,
} from './vendor/injected/hitTarget';
import type { RefactBuiltins } from './vendor/injected/utilityScript';
import { getAriaRole, getImplicitAriaRole } from './vendor/injected/roleUtils';
import { getElementAccessibleDescription, getElementAccessibleName } from './vendor/injected/roleUtils';
import {
  generateAriaTree,
  renderAriaTreeAsJSON,
  type AriaTreeOptions,
} from './vendor/injected/ariaSnapshot';
import { renderAriaSnapshotAsYaml } from './vendor/isomorphic/ariaSnapshotRenderer';
import { createRoleEngine } from './vendor/injected/roleSelectorEngine';
import {
  elementMatchesText,
  elementText,
  getElementLabels,
  matchesAttributePart,
  type ElementText,
  type TextMatcher,
} from './vendor/injected/selectorUtils';
import { normalizeWhiteSpace } from './vendor/isomorphic/stringUtils';

type ElementStateName = 'visible' | 'enabled' | 'editable' | 'checked' | 'unchecked' | 'mixed' | 'stable';

type ElementStates = Readonly<{
  visible: boolean;
  enabled: boolean;
  editable: boolean | null;
  checked: CheckedState | null;
  stable: boolean;
}>;

type RefactLocator = Readonly<{
  by: string;
  value?: string;
  role?: string;
  name?: string;
  description?: string;
  exact?: boolean;
  regex?: Readonly<{ source: string; flags?: string }>;
  name_regex?: Readonly<{ source: string; flags?: string }>;
  description_regex?: Readonly<{ source: string; flags?: string }>;
  checked?: boolean | 'mixed';
  pressed?: boolean | 'mixed';
  selected?: boolean;
  expanded?: boolean;
  disabled?: boolean;
  level?: number;
  include_hidden?: boolean;
  attribute?: string;
  nth?: number;
  within?: string;
}>;

type LocatorRegex = Readonly<{ source: string; flags?: string }>;

const injectedInstanceName = '__refact_injected__';
const bindingName = '__refact_binding';

type RefactGlobal = typeof globalThis & {
  [injectedInstanceName]?: RefactInjected;
  [bindingName]?: (payload: string) => void;
};

function createLocatorRegExp(value: LocatorRegex): RegExp {
  return new RegExp(value.source, value.flags ?? '');
}

function matchesRegExp(expression: RegExp, value: string): boolean {
  expression.lastIndex = 0;
  return expression.test(value);
}

function createLocatorTextMatcher(value: string, exact = false, regex?: LocatorRegex): { matcher: TextMatcher; kind: 'regex' | 'strict' | 'lax' } {
  if (regex) {
    const expression = createLocatorRegExp(regex);
    return { matcher: text => matchesRegExp(expression, text.normalized), kind: 'regex' };
  }
  const normalized = normalizeWhiteSpace(value);
  if (exact)
    return { matcher: text => text.normalized === normalized, kind: 'strict' };
  const folded = normalized.toLowerCase();
  return { matcher: text => text.normalized.toLowerCase().includes(folded), kind: 'lax' };
}

function queryAllPiercingShadow(scope: Document | Element): Element[] {
  const result: Element[] = [];
  const visit = (root: Document | Element | ShadowRoot) => {
    if (root instanceof Element)
      result.push(root);
    for (const element of root.querySelectorAll('*')) {
      result.push(element);
      if (element.shadowRoot)
        visit(element.shadowRoot);
    }
  };
  visit(scope);
  return result;
}

function queryDescendantsPiercingShadow(scope: Document | Element): Element[] {
  const elements = queryAllPiercingShadow(scope);
  if (scope instanceof Element)
    elements.shift();
  return elements;
}

function queryByAttribute(scope: Document | Element, attribute: string, value: string, exact = false, regex?: LocatorRegex): Element[] {
  if (!attribute)
    throw new Error('Test id attribute must not be empty');
  const expression = regex ? createLocatorRegExp(regex) : undefined;
  const expected = normalizeWhiteSpace(value);
  const expectedFolded = expected.toLowerCase();
  return queryDescendantsPiercingShadow(scope).filter(element => {
    const raw = element.getAttribute(attribute);
    if (raw === null)
      return false;
    const actual = normalizeWhiteSpace(raw);
    if (expression)
      return matchesRegExp(expression, actual);
    return exact ? actual === expected : actual.toLowerCase().includes(expectedFolded);
  });
}

function serializeRoleValue(value: string | undefined, regex: LocatorRegex | undefined, exact: boolean | undefined): string | undefined {
  if (regex) {
    const expression = createLocatorRegExp(regex);
    return `/${expression.source}/${expression.flags}`;
  }
  if (value === undefined)
    return undefined;
  return `${JSON.stringify(value)}${exact ? 's' : 'i'}`;
}

function serializeRoleSelector(locator: RefactLocator): string {
  const role = locator.role ?? '';
  const attributes: string[] = [];
  const add = (name: string, value: string | number | boolean | undefined) => {
    if (value !== undefined)
      attributes.push(`[${name}=${String(value)}]`);
  };
  add('checked', locator.checked);
  add('disabled', locator.disabled);
  add('selected', locator.selected);
  add('expanded', locator.expanded);
  add('include-hidden', locator.include_hidden);
  add('level', locator.level);
  add('name', serializeRoleValue(locator.name, locator.name_regex, locator.exact));
  add('description', serializeRoleValue(locator.description, locator.description_regex, locator.exact));
  add('pressed', locator.pressed);
  return `${role}${attributes.join('')}`;
}

export class RefactInjected {
  private readonly global: typeof globalThis;
  private readonly builtinSnapshot: RefactBuiltins;
  private readonly hitTargetController: HitTargetController;

  constructor(global: typeof globalThis, builtins: RefactBuiltins) {
    this.global = global;
    this.builtinSnapshot = builtins;
    this.hitTargetController = new HitTargetController(global, builtins);
  }

  version(): string {
    return 'playwright-1.63.0-next-refact-1';
  }

  builtins(): RefactBuiltins {
    return this.builtinSnapshot;
  }

  resolveSimple(cssSelector: string): Element | null {
    return this.global.document.querySelector(cssSelector);
  }

  resolveAll(locator: RefactLocator, scopeOverride?: Document | Element): Element[] {
    const document = this.global.document;
    const scope = scopeOverride ?? (locator.within ? document.querySelector(locator.within) : document);
    if (!scope)
      throw new Error('Scope selector not found');
    let elements: Element[];
    switch (locator.by) {
      case 'css':
        elements = Array.from(scope.querySelectorAll(locator.value ?? ''));
        break;
      case 'id': {
        const element = scope.querySelector(`#${CSS.escape(locator.value ?? '')}`);
        elements = element ? [element] : [];
        break;
      }
      case 'name':
        elements = Array.from(scope.querySelectorAll(`[name=${JSON.stringify(locator.value ?? '')}]`));
        break;
      case 'test_id':
        elements = queryByAttribute(scope, locator.attribute ?? 'data-testid', locator.value ?? '', locator.exact, locator.regex);
        break;
      case 'placeholder':
        elements = queryByAttribute(scope, 'placeholder', locator.value ?? '', locator.exact, locator.regex);
        break;
      case 'alt_text':
        elements = queryByAttribute(scope, 'alt', locator.value ?? '', locator.exact, locator.regex);
        break;
      case 'title':
        elements = queryByAttribute(scope, 'title', locator.value ?? '', locator.exact, locator.regex);
        break;
      case 'autocomplete':
        elements = Array.from(scope.querySelectorAll(`[autocomplete=${JSON.stringify(locator.value ?? '')}]`));
        break;
      case 'text': {
        const cache = new Map<Element | ShadowRoot, ElementText>();
        const { matcher, kind } = createLocatorTextMatcher(locator.value ?? '', locator.exact, locator.regex);
        elements = [];
        let lastDidNotMatchSelf: Element | null = null;
        for (const element of queryAllPiercingShadow(scope)) {
          if (kind === 'lax' && lastDidNotMatchSelf && lastDidNotMatchSelf.contains(element))
            continue;
          const matches = elementMatchesText(cache, element, matcher);
          if (matches === 'none')
            lastDidNotMatchSelf = element;
          if (matches === 'self')
            elements.push(element);
        }
        break;
      }
      case 'label': {
        const cache = new Map<Element | ShadowRoot, ElementText>();
        const { matcher } = createLocatorTextMatcher(locator.value ?? '', locator.exact, locator.regex);
        elements = queryDescendantsPiercingShadow(scope).filter(element =>
          getElementLabels(cache, element).some(label => matcher(label)),
        );
        break;
      }
      case 'role': {
        const selector = serializeRoleSelector(locator);
        elements = createRoleEngine(true).queryAll(scope, selector);
        break;
      }
      case 'xpath': {
        const result = document.evaluate(
          locator.value ?? '',
          scope,
          null,
          XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
          null,
        );
        elements = [];
        for (let index = 0; index < result.snapshotLength; index++) {
          const element = result.snapshotItem(index);
          if (element instanceof Element)
            elements.push(element);
        }
        break;
      }
      default:
        throw new Error(`Unknown locator strategy: ${locator.by}`);
    }
    if (locator.nth !== undefined)
      elements = elements.length > locator.nth ? [elements[locator.nth]] : [];
    return elements;
  }

  async elementState(element: Element, state: ElementStateName): Promise<Record<string, unknown>> {
    this.ensureConnected(element);
    if (state === 'visible') {
      const visible = isElementVisible(element);
      return { visible, matches: visible };
    }
    if (state === 'enabled') {
      const enabled = !getAriaDisabled(element);
      return { enabled, matches: enabled };
    }
    if (state === 'editable') {
      const editable = this.editableState(element);
      if (editable === null)
        throw new Error('Element is not an <input>, <textarea>, <select> or [contenteditable] and does not have a role allowing [aria-readonly]');
      return { editable, matches: editable };
    }
    if (state === 'checked' || state === 'unchecked' || state === 'mixed') {
      const checked = getCheckedState(element);
      if (checked === null)
        throw new Error('Not a checkbox or radio button');
      return {
        checked,
        matches: state === 'checked' ? checked === 'checked' : state === 'unchecked' ? checked === 'unchecked' : checked === 'mixed',
      };
    }
    if (state === 'stable') {
      const stable = await this.checkElementIsStable(element);
      return { stable, matches: stable };
    }
    throw new Error(`Unexpected element state "${state}"`);
  }

  async elementStates(element: Element): Promise<ElementStates> {
    return {
      visible: this.bestEffort(() => element.isConnected && isElementVisible(element), false),
      enabled: this.bestEffort(() => element.isConnected && !getAriaDisabled(element), false),
      editable: this.bestEffort(() => element.isConnected ? this.editableState(element) : null, null),
      checked: this.bestEffort(() => element.isConnected ? getCheckedState(element) : null, null),
      stable: await this.bestEffortStable(element),
    };
  }

  expectHitTarget(element: Element, point: HitTargetPoint): HitTargetResult {
    return this.hitTargetController.expectHitTarget(point, element);
  }

  installHitTargetInterceptor(
    element: Element,
    action: HitTargetAction,
    point: HitTargetPoint | undefined,
    blockAllEvents = false,
  ): Readonly<{ status: 'installed'; id: number }> | HitTargetResult {
    return this.hitTargetController.install(element, action, point, blockAllEvents);
  }

  takeHitTargetInterceptor(id: number): HitTargetResult {
    return this.hitTargetController.take(id);
  }

  private bestEffort<T>(read: () => T, fallback: T): T {
    try {
      return read();
    } catch {
      return fallback;
    }
  }

  private async bestEffortStable(element: Element): Promise<boolean> {
    try {
      return element.isConnected && await this.checkElementIsStable(element);
    } catch {
      return false;
    }
  }

  private editableState(element: Element): boolean | null {
    const readonly = getReadonly(element);
    return readonly === 'error' ? null : !getAriaDisabled(element) && !readonly;
  }

  private ensureConnected(element: Element): void {
    if (!element || !element.isConnected)
      throw new Error('Element is not attached to the DOM');
  }

  private async checkElementIsStable(element: Element): Promise<boolean> {
    const requestAnimationFrame = this.builtinSnapshot.requestAnimationFrame;
    const performanceNow = this.builtinSnapshot.performanceNow;
    let lastRect: { x: number; y: number; width: number; height: number } | undefined;
    let lastTime = 0;
    return await new Promise<boolean>((resolve, reject) => {
      const check = () => {
        try {
          this.ensureConnected(element);
          const time = performanceNow();
          if (lastTime && time - lastTime < 15) {
            requestAnimationFrame(check);
            return;
          }
          lastTime = time;
          const clientRect = element.getBoundingClientRect();
          const rect = { x: clientRect.x, y: clientRect.y, width: clientRect.width, height: clientRect.height };
          if (lastRect) {
            resolve(
              rect.x === lastRect.x &&
              rect.y === lastRect.y &&
              rect.width === lastRect.width &&
              rect.height === lastRect.height,
            );
            return;
          }
          lastRect = rect;
          requestAnimationFrame(check);
        } catch (error) {
          reject(error);
        }
      };
      try {
        requestAnimationFrame(check);
      } catch (error) {
        reject(error);
      }
    });
  }

  dispatchBinding(name: string, payload: unknown): void {
    const global = this.global as RefactGlobal;
    const binding = global[bindingName];
    const stringify = this.builtinSnapshot.jsonStringify;
    if (!binding)
      throw new Error(`${bindingName} is not installed`);
    binding(stringify({ name, payload }));
  }

  getImplicitRole(element: Element): string {
    this.ensureConnected(element);
    return getImplicitAriaRole(element) ?? 'generic';
  }

  computeRole(element: Element): string {
    this.ensureConnected(element);
    return getAriaRole(element) ?? 'generic';
  }

  getAccessibleName(element: Element, includeHidden = false): string {
    this.ensureConnected(element);
    // Matching normalization, case folding, substring, and regex semantics belong to locator consumers.
    return getElementAccessibleName(element, includeHidden).text;
  }

  getAccessibleDescription(element: Element): string {
    this.ensureConnected(element);
    return getElementAccessibleDescription(element, false).text;
  }

  ariaSnapshot(element: Element | null, options: AriaTreeOptions): Record<string, unknown> {
    const root = element ?? this.global.document.body ?? this.global.document.documentElement;
    this.ensureConnected(root);
    const tree = generateAriaTree(root, options);
    const { json } = renderAriaTreeAsJSON(tree, options);
    const nodes: Record<string, unknown>[] = [];
    if (options.boxes) {
      const visit = (node: (typeof json)[number] | string) => {
        if (typeof node === 'string')
          return;
        if (node.box)
          nodes.push({ role: node.role, name: node.name, ref: node.ref, box: node.box });
        for (const child of node.children ?? [])
          visit(child);
      };
      for (const node of json)
        visit(node);
    }
    return { yaml: renderAriaSnapshotAsYaml(json), nodes };
  }
}

export function bootstrapRefactInjected(
  global: typeof globalThis,
  builtins: RefactBuiltins,
): RefactInjected {
  const refactGlobal = global as RefactGlobal;
  const existing = refactGlobal[injectedInstanceName];
  if (existing)
    return existing;
  const injected = new RefactInjected(global, builtins);
  refactGlobal[injectedInstanceName] = injected;
  return injected;
}

import { SelectorEvaluatorImpl, sortInDOMOrder } from './vendor/injected/selectorEvaluator';
import { kLayoutSelectorNames, layoutSelectorScore } from './vendor/injected/layoutSelectorUtils';
import { XPathEngine } from './vendor/injected/xpathSelectorEngine';
import { generateSelector, type GenerateSelectorOptions } from './vendor/injected/selectorGenerator';
import type { SelectorEngine, SelectorRoot } from './vendor/injected/selectorEngine';
import {
  parseSelector,
  parseAttributeSelector,
  stringifySelector,
  type NestedSelectorBody,
  type ParsedSelector,
  type ParsedSelectorPart,
} from './vendor/isomorphic/selectorParser';

type SelectorQueryScope = Document | Element | ShadowRoot;

const selectorEvaluator = new SelectorEvaluatorImpl();
const selectorEngines = new Map<string, SelectorEngine>([
  ['css', {
    queryAll(root: SelectorRoot, selector: unknown): Element[] {
      return selectorEvaluator.query({ scope: root as Document | Element, pierceShadow: true }, selector);
    },
  }],
  ['xpath', XPathEngine],
]);

function createInternalTextMatcher(selector: string): TextMatcher {
  if (selector.startsWith('/') && selector.lastIndexOf('/') > 0) {
    const lastSlash = selector.lastIndexOf('/');
    const expression = new RegExp(selector.slice(1, lastSlash), selector.slice(lastSlash + 1));
    return text => matchesRegExp(expression, text.full);
  }
  const exact = selector.endsWith('s');
  const value = normalizeWhiteSpace(JSON.parse(selector.slice(0, -1)) as string);
  if (exact)
    return text => text.normalized === value;
  const folded = value.toLowerCase();
  return text => text.normalized.toLowerCase().includes(folded);
}

function createInternalAttributeEngine(testId: boolean): SelectorEngine {
  return {
    queryAll(root: SelectorRoot, selector: string): Element[] {
      const parsed = parseAttributeSelector(selector, true);
      if (parsed.name || parsed.attributes.length !== 1)
        throw new Error(`Malformed ${testId ? 'test id' : 'attribute'} selector: ${selector}`);
      const attribute = parsed.attributes[0];
      const names = testId ? attribute.name.split(',') : [attribute.name];
      return queryDescendantsPiercingShadow(root as Document | Element).filter(element =>
        names.some(name => {
          const value = element.getAttribute(name);
          return value !== null && matchesAttributePart(value, attribute);
        }),
      );
    },
  };
}

selectorEngines.set('internal:role', createRoleEngine(true));
selectorEngines.set('internal:text', {
  queryAll(root: SelectorRoot, selector: string): Element[] {
    const matcher = createInternalTextMatcher(selector);
    return queryAllPiercingShadow(root as Document | Element).filter(element =>
      elementMatchesText(selectorEvaluator._cacheText, element, matcher) === 'self',
    );
  },
});
selectorEngines.set('internal:has-text', {
  queryAll(root: SelectorRoot, selector: string): Element[] {
    if (!(root instanceof Element))
      return [];
    return createInternalTextMatcher(selector)(elementText(selectorEvaluator._cacheText, root)) ? [root] : [];
  },
});
selectorEngines.set('internal:label', {
  queryAll(root: SelectorRoot, selector: string): Element[] {
    const matcher = createInternalTextMatcher(selector);
    return queryDescendantsPiercingShadow(root as Document | Element).filter(element =>
      getElementLabels(selectorEvaluator._cacheText, element).some(matcher),
    );
  },
});
selectorEngines.set('internal:attr', createInternalAttributeEngine(false));
selectorEngines.set('internal:testid', createInternalAttributeEngine(true));

function querySelectorPart(part: ParsedSelectorPart, root: SelectorRoot): Element[] {
  const engine = selectorEngines.get(part.name);
  if (!engine)
    throw new Error(`Unknown selector engine "${part.name}"`);
  return engine.queryAll(root, part.body);
}

function queryLayoutSelector(
  elements: Set<Element>,
  part: ParsedSelectorPart,
  originalRoot: SelectorQueryScope,
): Set<Element> {
  const body = part.body as NestedSelectorBody;
  const inner = queryParsedSelector(body.parsed, originalRoot);
  const matches: { element: Element; score: number }[] = [];
  for (const element of elements) {
    const score = layoutSelectorScore(part.name as typeof kLayoutSelectorNames[number], element, inner, body.distance);
    if (score !== undefined)
      matches.push({ element, score });
  }
  matches.sort((left, right) => left.score - right.score);
  return new Set(matches.map(match => match.element));
}

function queryParsedSelector(selector: ParsedSelector, root: SelectorQueryScope): Element[] {
  if (selector.capture !== undefined) {
    const captured: ParsedSelector = { parts: selector.parts.slice(0, selector.capture + 1) };
    if (selector.capture < selector.parts.length - 1) {
      const parsed: ParsedSelector = { parts: selector.parts.slice(selector.capture + 1) };
      captured.parts.push({
        name: 'internal:has',
        body: { parsed },
        source: stringifySelector(parsed),
      });
    }
    return queryParsedSelector(captured, root);
  }
  selectorEvaluator.begin();
  try {
    let roots = new Set<Element>([root as Element]);
    for (const part of selector.parts) {
      if (part.name === 'nth') {
        const index = Number(part.body);
        roots = Number.isInteger(index) && index >= 0 && index < roots.size
          ? new Set([[...roots][index]])
          : new Set();
      } else if (part.name === 'internal:has') {
        roots = new Set([...roots].filter(element =>
          queryParsedSelector((part.body as NestedSelectorBody).parsed, element).length > 0,
        ));
      } else if (part.name === 'internal:and') {
        const andElements = queryParsedSelector((part.body as NestedSelectorBody).parsed, root);
        roots = new Set(andElements.filter(element => roots.has(element)));
      } else if (part.name === 'internal:or') {
        const orElements = queryParsedSelector((part.body as NestedSelectorBody).parsed, root);
        roots = new Set(sortInDOMOrder(new Set([...roots, ...orElements])));
      } else if (kLayoutSelectorNames.includes(part.name as typeof kLayoutSelectorNames[number])) {
        roots = queryLayoutSelector(roots, part, root);
      } else {
        const next = new Set<Element>();
        for (const queryRoot of roots) {
          for (const element of querySelectorPart(part, queryRoot))
            next.add(element);
        }
        roots = next;
      }
    }
    return [...roots];
  } finally {
    selectorEvaluator.end();
  }
}

Object.defineProperty(RefactInjected.prototype, 'querySelectorAll', {
  value(selectorChain: string, scope?: SelectorQueryScope): Element[] {
    return queryParsedSelector(parseSelector(selectorChain), scope ?? globalThis.document);
  },
});

const _lastAriaSnapshotForQuery = new WeakMap<RefactInjected, Map<string, { element: Element, nameFromContentRefs: string[] }>>();

Object.defineProperty(RefactInjected.prototype, 'ariaSnapshot', {
  value(this: RefactInjected, element: Element | null, options: AriaTreeOptions): Record<string, unknown> {
    const root = element ?? globalThis.document.body ?? globalThis.document.documentElement;
    if (!root.isConnected)
      throw new Error('Element is not connected to a document');
    const tree = generateAriaTree(root, options);
    const { json } = renderAriaTreeAsJSON(tree, options);
    _lastAriaSnapshotForQuery.set(this, tree.info);
    const nodes: Record<string, unknown>[] = [];
    if (options.boxes || options.refs) {
      const visit = (node: (typeof json)[number] | string) => {
        if (typeof node === 'string')
          return;
        if (node.ref || node.box)
          nodes.push({ role: node.role, name: node.name, ref: node.ref, box: node.box });
        for (const child of node.children ?? [])
          visit(child);
      };
      for (const node of json)
        visit(node);
    }
    return { yaml: renderAriaSnapshotAsYaml(json), nodes };
  },
});

Object.defineProperty(RefactInjected.prototype, 'resolveAriaRef', {
  value(this: RefactInjected, reference: string): Element {
    const result = _lastAriaSnapshotForQuery.get(this)?.get(reference);
    if (!result)
      throw new Error(`REF_UNKNOWN: ref ${reference} is unknown; take a fresh snapshot`);
    if (!result.element.isConnected)
      throw new Error(`REF_DETACHED: ref ${reference} is detached; take a fresh snapshot`);
    return result.element;
  },
});

Object.defineProperty(RefactInjected.prototype, 'generateLocator', {
  value(element: Element, options: GenerateSelectorOptions): string {
    if (!element.isConnected)
      throw new Error('Element is not attached to the DOM');
    const runtime = {
      _evaluator: selectorEvaluator,
      parseSelector,
      querySelector: (selector: ParsedSelector, root: Document | Element) =>
        queryParsedSelector(selector, root)[0],
      querySelectorAll: (selector: ParsedSelector, root: Document | Element) =>
        queryParsedSelector(selector, root),
    };
    return generateSelector(runtime, element, options).selector;
  },
});
