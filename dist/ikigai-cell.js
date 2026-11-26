// <ikigai-cell>: a runnable ikigai cell for any page (a slide deck, a course page, the Book).
//
// A cell shows its code (its own text content), a Run button and an output region. Every
// cell on a page runs on ONE kernel, built once from the page's declaration, a JSON object
// in a <script type="application/ikigai-kernel+json"> block (see `cell` in src/lib.rs and
// the README): the modules it holds, the resources the page serves, and the capability
// every cell runs under, sealed. No declaration is the empty one: nothing bound, nothing
// granted.
//
// This file is also the code underneath the demo page's <ikigai-cli>: `renderAction` turns
// the kernel's `{ kind, text, cache }` answer into transcript lines for both.
//
// It is the SOURCE. `scripts/package-cell.sh` copies it, its stylesheet, the wasm and its
// glue into a directory of content-hashed files a host copies as is; every
// cache-busting `?v=` specifier below is rewritten to a hashed name there. Nothing is fetched
// from any other host.
// The glue is imported when a cell first runs, not with this module: the demo page imports
// `renderAction` from here and must not load a second copy of the glue it already holds.
const GLUE_URL = new URL('./ikigai_web_demo.js?v=__BUILD__', import.meta.url);
const WASM_URL = new URL('./ikigai_web_demo_bg.wasm?v=__BUILD__', import.meta.url);
const CSS_URL = new URL('./ikigai-cell.css?v=__BUILD__', import.meta.url);
const DECLARATION = 'script[type="application/ikigai-kernel+json"]';

/// The transcript lines one kernel answer renders as: `{ text, cls }` with `cls` one of
/// `out`, `error`, `help`, `cache`, `hint`. `clear` and `quit` come back as `action`, for
/// the caller to act on. Shared by <ikigai-cli> (the demo's terminal) and <ikigai-cell>.
export function renderAction(r) {
  if (r.kind === 'noop') return { lines: [] };
  if (r.kind === 'clear' || r.kind === 'quit') return { action: r.kind, lines: [] };
  const lines = [];
  if (r.text) lines.push({ text: r.text, cls: r.kind === 'error' ? 'error' : (r.kind === 'help' ? 'help' : 'out') });
  if (r.cache) lines.push({ text: r.cache, cls: 'cache' });
  return { lines };
}

function emit(target, name, detail) {
  target.dispatchEvent(new CustomEvent(name, { bubbles: true, composed: true, detail }));
}

// The page's one kernel: the wasm instantiated once, the declaration read once.
let kernel = null;
let glue = null;
function pageKernel() {
  if (!kernel) {
    kernel = (async () => {
      const blocks = document.querySelectorAll(DECLARATION);
      if (blocks.length > 1) {
        throw new Error(`this page has ${blocks.length} kernel declarations; a page declares one`);
      }
      glue = await import(GLUE_URL.href);
      await glue.default({ module_or_path: WASM_URL });
      return JSON.parse(glue.cellKernel(blocks[0] ? blocks[0].textContent : ''));
    })().then(
      (declaration) => { emit(document, 'ikigai-kernel-ready', { declaration }); return declaration; },
      (error) => {
        const message = String(error && error.message ? error.message : error);
        emit(document, 'ikigai-kernel-error', { message });
        throw new Error(message);
      },
    );
  }
  return kernel;
}

function ensureStyles() {
  if (document.querySelector('link[data-ikigai-cell]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = CSS_URL.href;
  link.dataset.ikigaiCell = '';
  document.head.appendChild(link);
}

let cells = 0;
// Runs are serialized page-wide, in the order they were asked for, so cells that run
// together (`autorun`, or a host running a slide's cells in turn) see each other's writes
// in document order.
let queue = Promise.resolve();

export class IkigaiCell extends HTMLElement {
  connectedCallback() {
    if (this._wired) return;
    this._wired = true;
    ensureStyles();
    const n = ++cells;
    const name = this.getAttribute('label') || `ikigai cell ${n}`;
    const code = this.textContent.replace(/^\s*\n/, '').replace(/\s+$/, '');
    const ids = { hint: `ikc-hint-${n}` };
    this.replaceChildren();
    const frame = document.createElement('div');
    frame.className = 'ikc-frame';
    frame.setAttribute('role', 'group');
    frame.setAttribute('aria-label', name);
    this.code = document.createElement('textarea');
    this.code.className = 'ikc-code';
    this.code.value = code;
    this.code.rows = Math.max(1, code.split('\n').length);
    this.code.spellcheck = false;
    this.code.setAttribute('autocapitalize', 'off');
    this.code.setAttribute('autocomplete', 'off');
    this.code.setAttribute('aria-label', `${name}: code`);
    this.code.setAttribute('aria-describedby', ids.hint);
    this.code.readOnly = this.hasAttribute('readonly');
    const bar = document.createElement('div');
    bar.className = 'ikc-bar';
    this.button = document.createElement('button');
    this.button.type = 'button';
    this.button.className = 'ikc-run';
    this.button.textContent = 'Run';
    this.button.setAttribute('aria-describedby', ids.hint);
    const hint = document.createElement('span');
    hint.className = 'ikc-hint';
    hint.id = ids.hint;
    hint.textContent = 'Ctrl+Enter runs · Esc leaves the cell';
    bar.append(this.button, hint);
    this.output = document.createElement('div');
    this.output.className = 'ikc-output';
    this.output.setAttribute('role', 'status');
    this.output.setAttribute('aria-live', 'polite');
    this.output.setAttribute('aria-label', `${name}: output`);
    frame.append(this.code, bar, this.output);
    this.append(frame);

    this.button.addEventListener('click', () => this.run());
    // A focused cell keeps its keys: nothing typed in it reaches the host (reveal.js and
    // the like listen on the document and would turn arrows into slide changes). Escape
    // hands focus back to the host instead.
    for (const type of ['keydown', 'keyup', 'keypress']) {
      this.addEventListener(type, (e) => this.onKey(e));
    }
    if (this.hasAttribute('autorun')) this.run();
  }

  onKey(e) {
    e.stopPropagation();
    if (e.type !== 'keydown') return;
    if (e.key === 'Escape') {
      e.preventDefault();
      this.leave();
    } else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      this.run();
    }
  }

  // Give focus back to the host: the element named by `escape-to` (an id), else the
  // nearest focusable ancestor, else the document itself.
  leave() {
    const named = this.getAttribute('escape-to');
    let target = named ? document.getElementById(named) : null;
    if (!target) {
      for (let el = this.parentElement; el; el = el.parentElement) {
        if (el.matches('[tabindex]')) { target = el; break; }
      }
    }
    if (target) target.focus();
    if (this.contains(document.activeElement)) document.activeElement.blur();
    emit(this, 'ikigai-cell-escape', {});
  }

  line(text, cls) {
    const pre = document.createElement('pre');
    pre.className = 'ikc-line ikc-' + cls;
    pre.textContent = text;
    this.output.appendChild(pre);
  }

  /// Run the cell's code; resolves to `{ ok, results }`, as the `ikigai-cell-result` event
  /// carries it. A run asked for while this cell is running is the same run.
  run() {
    if (!this._running) {
      this._running = true;
      this._run = queue.then(() => this.runNow());
      queue = this._run.catch(() => {});
    }
    return this._run;
  }

  async runNow() {
    this.button.setAttribute('aria-busy', 'true');
    this.output.setAttribute('aria-busy', 'true');
    this.output.classList.remove('ikc-fresh');
    const code = this.code.value;
    const results = [];
    const fragment = [];
    let ok = true;
    try {
      await pageKernel();
      // Every run starts a fresh session at the page's declared capability: a `cap`
      // narrowing or an open `:lisp` buffer lasts for this run only. The kernel, and what
      // earlier cells wrote to it, stays.
      glue.cellBeginRun();
      const commands = JSON.parse(glue.cellCommands(code));
      emit(this, 'ikigai-cell-run', { code, commands });
      for (const command of commands) {
        const r = JSON.parse(await glue.cellEvalAsync(command));
        results.push({ command, kind: r.kind, text: r.text, cache: r.cache });
        if (commands.length > 1) fragment.push({ text: '› ' + command, cls: 'echo' });
        const { action, lines } = renderAction(r);
        if (action === 'clear') fragment.length = 0;
        if (action === 'quit') fragment.push({ text: '(quit does nothing in a cell)', cls: 'hint' });
        fragment.push(...lines);
        if (r.kind === 'error') {
          ok = false;
          if (command !== commands[commands.length - 1]) {
            fragment.push({ text: 'stopped: the lines after this one did not run', cls: 'hint' });
          }
          break;
        }
      }
      if (!commands.length) fragment.push({ text: '(nothing to run)', cls: 'hint' });
    } catch (error) {
      ok = false;
      fragment.push({ text: 'the kernel could not run this cell: ' + (error && error.message ? error.message : error), cls: 'error' });
    }
    this.output.replaceChildren();
    for (const { text, cls } of fragment) this.line(text, cls);
    this.output.classList.add('ikc-fresh');
    this.output.removeAttribute('aria-busy');
    this.button.removeAttribute('aria-busy');
    this._running = false;
    emit(this, 'ikigai-cell-result', { ok, results });
    return { ok, results };
  }
}

if (!customElements.get('ikigai-cell')) customElements.define('ikigai-cell', IkigaiCell);
