/* Claw website. No framework, no build step. */
(() => {
"use strict";

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
const sleep = ms => new Promise(r => setTimeout(r, ms));
const esc = s => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/* ───────────────────────── highlighter ───────────────────────── */
const KW = new Set("fn type use pub const mut own if else for in break continue match ret fail test defer and or not true false none".split(" "));
const CONTRACT = new Set(["need", "sure"]);
const UNSAFE = new Set(["claw", "extern", "alloc", "free", "cast"]);
const TYPES = new Set("int i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 bool str f64x2 f32x4 i32x4".split(" "));

function hlLine(line) {
  let out = "", i = 0;
  const n = line.length;
  while (i < n) {
    const c = line[i], rest = line.slice(i);
    if (rest.startsWith("//")) { out += `<span class="c">${esc(rest)}</span>`; break; }
    if (c === '"') {
      let j = i + 1;
      while (j < n && line[j] !== '"') j++;
      const body = esc(line.slice(i, j + 1)).replace(/\{([^}]*)\}/g, '<span class="ip">{$1}</span>');
      out += `<span class="s">${body}</span>`; i = j + 1; continue;
    }
    if (c === "'") {
      const j = line.indexOf("'", i + 1);
      out += `<span class="s">${esc(line.slice(i, j + 1))}</span>`; i = j + 1; continue;
    }
    if (c === "+" && /^\+(mem|io)\b/.test(rest)) {
      const m = rest.match(/^\+(mem|io)/)[0];
      out += `<span class="e">${m}</span>`; i += m.length; continue;
    }
    if (/[0-9]/.test(c)) {
      const m = rest.match(/^[0-9][0-9_.]*/)[0].replace(/\.\.$/, "");
      out += `<span class="n">${m}</span>`; i += m.length; continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      const m = rest.match(/^[A-Za-z_][A-Za-z0-9_]*/)[0];
      const after = line[i + m.length];
      let cls = "";
      if (UNSAFE.has(m)) cls = "u";
      else if (CONTRACT.has(m)) cls = "ct";
      else if (KW.has(m)) cls = "k";
      else if (TYPES.has(m) || /^[A-Z]/.test(m)) cls = "t";
      else if (after === "(" || after === "[") cls = "f";
      out += cls ? `<span class="${cls}">${m}</span>` : m;
      i += m.length; continue;
    }
    // pointer: unary * or & directly before a name, inside claw code
    if (c === "*" && /[A-Za-z(]/.test(line[i + 1] || "") && !/[A-Za-z0-9_)\]]/.test(line[i - 1] || "")) {
      out += `<span class="u">*</span>`; i++; continue;
    }
    if ("{}()[],:".includes(c)) { out += `<span class="p">${esc(c)}</span>`; i++; continue; }
    out += esc(c); i++;
  }
  return out;
}

function renderCode(el, src, { swap = false } = {}) {
  const lines = src.replace(/\n$/, "").split("\n");
  el.innerHTML = lines.map((l, k) =>
    `<div class="ln" data-l="${k + 1}" style="--i:${k}"><span class="no">${k + 1}</span><span class="src">${hlLine(l) || " "}</span></div>`
  ).join("");
  el.classList.remove("why");
  if (swap) { el.classList.remove("swap"); void el.offsetWidth; el.classList.add("swap"); }
}
const lineEl = (el, n) => el.querySelector(`.ln[data-l="${n}"]`);
function clearMarks(el) {
  el.classList.remove("why");
  $$(".ln", el).forEach(l => l.classList.remove("hl", "bad", "keep", "morph", "new"));
  $$(".anno", el).forEach(a => a.remove());
}
function anno(el, n, text, hot) {
  const l = lineEl(el, n); if (!l) return;
  const a = document.createElement("span");
  a.className = "anno" + (hot ? " hot" : "");
  a.textContent = text;
  l.querySelector(".src").appendChild(a);
}

/* ───────────────────────── terminal helpers ───────────────────────── */
function promptHTML(cmd, failed) {
  return `<span class="pr">~/claw</span><span class="pc${failed ? " fail" : ""}">❯</span>${esc(cmd)}`;
}
function fmtOut(text, cls) {
  let h = esc(text);
  h = h.replace(/^([\w.]+\.claw:\d+:\d+)/, '<span class="loc">$1</span>');
  h = h.replace(/\b(E\d{4})\b/g, '<span class="ecode">$1</span>');
  return `<div class="tl ${cls || ""}">${h || " "}</div>`;
}

async function typeInto(node, text, speed = 30) {
  node.classList.add("typing");
  const base = node.innerHTML;
  for (let k = 1; k <= text.length; k++) {
    node.innerHTML = base + esc(text.slice(0, k));
    await sleep(speed + Math.random() * speed * .8);
  }
  node.classList.remove("typing");
}

/* ───────────────────────── sources (real files, real output from claw 0.10) ───────────────────────── */
const SRC = {
  "avg.claw":
`fn avg(xs []int) int
    need len(xs) > 0
{
    mut total := 0
    for x in xs { total += x }
    ret total / len(xs)
}

fn main() {
    print(avg([4, 8, 15]))
    print(avg([]))
}`,
  "pick.claw":
`fn pick(xs []int, i int) int {
    ret xs[i]
}`,
  "pick.claw+need":
`fn pick(xs []int, i int) int
    need i >= 0 and i < len(xs)
{
    ret xs[i]
}`,
  "move.claw":
`fn main() {
    a []int = [1, 2, 3]
    b := a
    print(len(a) + len(b))
}`,
  "move.claw+fix":
`fn main() {
    a []int = [1, 2, 3]
    b := copy(a)
    print(len(a) + len(b))
}`,
  "pets.claw":
`fn pets() []str {
    mut a []str = []
    push(&a, "cat")
    push(&a, "dog")
    push(&a, "bird")
    ret a                    // handed over, no copy
}

fn total(xs []int) int {
    mut s := 0
    for x in xs { s += x }
    ret s
}

fn main() {
    a := pets()
    b := copy(a)             // a copy is always explicit
    print("{len(a)} {len(b)}")
    print(join(a, " "))
    print(total([1, 2, 3]))
}                            // a and b are freed here`,
  "paw_and_claw.claw":
`// expect: 6
// expect: 2
// Two gears in one language: \`fn\` is safe and automatic, \`claw fn\` is C.
extern fn strlen(s *u8) u64 // any libSystem function

claw fn sum3() int {
    p := alloc[int](3)
    defer free(p) // runs when the block ends
    p[0] = 1
    p[1] = 2
    q := p + 2
    *q = 3
    ret p[0] + p[1] + p[2]
}

claw fn length_of_hi() u64 {
    ret strlen("hi")
}

fn main() {
    print(sum3()) // safe code calls claw code
    print(length_of_hi())
}`,
};

const OUT = {
  avgRun: [["", "9"], ["err", "trap: need failed: `len(xs) > 0` in avg at avg.claw:11:11"], ["err", "  left = 0"], ["err", "  xs = []"]],
  avgWhy: [["err", "FAIL need failed: `len(xs) > 0` in avg at avg.claw:11:11"], ["", "why    avg.claw:2  need len(xs) > 0"], ["", "       avg.claw:11  print(avg([]))    left = 0, xs = []"]],
  avgBuild: [["ok", "checks: 2 of 4 removed, 4 calls inlined"]],
  pickTest: [["err", "FAIL pick: index out of bounds at pick.claw:2:9"], ["", "  input  pick([], 0)"], ["", "  why    pick.claw:1  fn pick(xs []int, i int) int {    i = 0"], ["", "         pick.claw:2  ret xs[i]    index = 0, len = 0"], ["ok", "  fix    add `need i >= 0 and i < len(xs)` to pick"], ["note", "auto: 1 functions, 1 inputs, 1 failed"]],
  pickTestOk: [["ok", "auto: 1 functions, 4000 inputs, 0 failed"]],
  pickRun: [["err", "pick.claw:1:1 E0109 no `fn main()`; every program needs one"]],
  moveCheck: [["err", "move.claw:4:15 E0151 `a` was moved away at line 3; fix: write `copy(a)` there"]],
  petsRun: [["", "3 3"], ["", "cat dog bird"], ["", "6"]],
  petsSig: [["ok", "fn pets() []str +mem"], ["ok", "fn total(xs []int) int"], ["ok", "fn main() +mem +io"]],
  petsTest: [["note", "note: total overflows for total([4611686018427387903, 2, 4611686018427387903]); add a `need` if that input is not meant to work"], ["ok", "auto: 2 functions, 8000 inputs, 0 failed"]],
  audit: [["err", "extern   paw_and_claw.claw:4  strlen  effects taken on trust"], ["err", "claw fn  paw_and_claw.claw:6  sum3  alloc free index arith deref"], ["err", "claw fn  paw_and_claw.claw:16  length_of_hi  extern"], ["", "audit: 2 claw fn, 1 extern, 0 checks not in `claw build`; std has 17 claw fn, not listed"]],
  pawRun: [["", "6"], ["", "2"]],
  help: `claw run <file.claw>            compile with all checks and run
claw test <file.claw>           run the \`test\` blocks, then try every function without \`+io\` on generated inputs
claw why <file.claw>            run again and say which lines led to the failure
claw fix <file.claw>            apply the compiler's safe patches
claw build <file.claw> [-o out] optimized executable; contracts with calls are left out
    --fast                      remove every run-time check: speed like C, risk like C
    --stats                     say how many checks were proved unnecessary
claw check <file.claw>          report errors only
claw card                       the language on one page
claw sig <file.claw> | std      only the signatures, with the effects worked out
claw ctx <file.claw> <fn>       one function and what it takes to change it
claw audit <file.claw>          every place the compiler has to trust
claw fmt <file.claw> [--check]  put a file into the one standard format
claw lsp                        language server on stdin and stdout, for editors`.split("\n").map(l => ["", l]),
};

/* ───────────────────────── hero: scratched dot field ───────────────────────── */
function heroCanvas() {
  const cv = $("#scratch"); if (!cv) return;
  const ctx = cv.getContext("2d");
  let W, H, dpr, pts = [], curves = [], t0 = performance.now(), visible = true;
  const mouse = { x: -9999, y: -9999, tx: -9999, ty: -9999 };

  function build() {
    dpr = Math.min(devicePixelRatio || 1, 2);
    W = cv.clientWidth; H = cv.clientHeight;
    cv.width = W * dpr; cv.height = H * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const mobile = W < 700;
    const g = mobile ? 15 : 19;
    const maxW = mobile ? 15 : Math.min(30, W / 52);
    // three claw marks, top-right to bottom-left, slightly curved
    const ox = mobile ? W * .52 : W * .6, span = mobile ? W * .17 : Math.min(W * .095, 150);
    curves = [0, 1, 2].map(k => {
      const x0 = ox + k * span + (mobile ? 40 : 120), y0 = H * (.1 + k * .03);
      const x1 = ox + k * span - (mobile ? 30 : 70), y1 = H * (.86 - (k === 1 ? 0 : .06));
      const cx = (x0 + x1) / 2 + (mobile ? 18 : 40), cy = (y0 + y1) / 2;
      const s = [];
      for (let q = 0; q <= 80; q++) {
        const u = q / 80, a = (1 - u) * (1 - u), b = 2 * (1 - u) * u, c = u * u;
        s.push({ x: a * x0 + b * cx + c * x1, y: a * y0 + b * cy + c * y1, u });
      }
      return { s, w: maxW * (k === 1 ? 1.1 : .9), delay: k * 140 };
    });
    pts = [];
    for (let y = g / 2; y < H; y += g) for (let x = g / 2; x < W; x += g) {
      let best = null;
      curves.forEach((cu, ci) => {
        for (let q = 0; q < cu.s.length; q++) {
          const p = cu.s[q], dx = x - p.x, dy = y - p.y, d = dx * dx + dy * dy;
          if (!best || d < best.d) best = { d, ci, q, dx, dy };
        }
      });
      const cu = curves[best.ci], u = cu.s[best.q].u, d = Math.sqrt(best.d);
      const w = cu.w * Math.pow(Math.sin(Math.PI * u), .7);
      const inf = w + 26;
      if (d < inf) {
        const nx = d ? best.dx / d : 0, ny = d ? best.dy / d : 0;
        const push = d < w ? (w - d) + 2 : (inf - d) * .12;
        pts.push({ x, y, ci: best.ci, u, nx, ny, push, edge: Math.max(0, 1 - Math.abs(d - w) / 10) });
      } else pts.push({ x, y, ci: -1 });
    }
  }

  // the ember inside the cuts lives on its own canvas, blurred by CSS (cheap: redrawn only while it grows)
  const gcv = document.createElement("canvas");
  gcv.className = "hero-canvas hero-ember"; gcv.setAttribute("aria-hidden", "true");
  cv.before(gcv);
  const gctx = gcv.getContext("2d");
  let lastE = null, dirty = true;

  function drawEmber(ease) {
    gcv.width = W * dpr; gcv.height = H * dpr;
    gctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    curves.forEach((cu, ci) => {
      const e = ease[ci]; if (e <= 0) return;
      const last = Math.floor(e * (cu.s.length - 1));
      const side = (q, sgn) => {
        const p = cu.s[q], pn = cu.s[Math.min(q + 1, cu.s.length - 1)], pp = cu.s[Math.max(q - 1, 0)];
        const tx = pn.x - pp.x, ty = pn.y - pp.y, tl = Math.hypot(tx, ty) || 1;
        const w = cu.w * .55 * Math.pow(Math.sin(Math.PI * p.u), .9);
        return [p.x - sgn * ty / tl * w, p.y + sgn * tx / tl * w];
      };
      gctx.beginPath();
      for (let q = 0; q <= last; q++) { const [X, Y] = side(q, 1); q ? gctx.lineTo(X, Y) : gctx.moveTo(X, Y); }
      for (let q = last; q >= 0; q--) { const [X, Y] = side(q, -1); gctx.lineTo(X, Y); }
      gctx.closePath();
      const a = cu.s[0], z = cu.s[cu.s.length - 1];
      const gr = gctx.createLinearGradient(a.x, a.y, z.x, z.y);
      gr.addColorStop(0, "rgba(255,90,31,0)");
      gr.addColorStop(.35, "rgba(255,120,60,.95)");
      gr.addColorStop(.7, "rgba(255,90,31,.7)");
      gr.addColorStop(1, "rgba(255,90,31,0)");
      gctx.fillStyle = gr;
      gctx.fill();
    });
  }

  const BUCKETS = 14;
  const bucket = Array.from({ length: BUCKETS }, () => []);
  function draw(now) {
    requestAnimationFrame(draw);
    if (!visible) return;
    const t = now - t0;
    const mx = mouse.x, my = mouse.y;
    mouse.x += (mouse.tx - mouse.x) * .14; mouse.y += (mouse.ty - mouse.y) * .14;
    const moving = Math.abs(mouse.x - mx) + Math.abs(mouse.y - my) > .2;
    const ease = curves.map(cu => {
      const p = reduced ? 1 : Math.min(1, Math.max(0, (t - 500 - cu.delay) / 1100));
      return 1 - Math.pow(1 - p, 3);
    });
    const growing = ease.some((e, i) => !lastE || e !== lastE[i]);
    if (growing) { drawEmber(ease); lastE = ease; }
    if (!growing && !moving && !dirty) return;
    dirty = false;

    ctx.clearRect(0, 0, W, H);
    for (const b of bucket) b.length = 0;
    for (const p of pts) {
      let x = p.x, y = p.y, a = .11, r = 1;
      if (p.ci >= 0 && ease[p.ci] > p.u) {
        const k = Math.min(1, (ease[p.ci] - p.u) * 8);
        x += p.nx * p.push * k; y += p.ny * p.push * k;
        a += p.edge * .45 * k; r += p.edge * .5 * k;
      }
      const dx = x - mouse.x, dy = y - mouse.y, d2 = dx * dx + dy * dy;
      if (d2 < 25921) {
        const d = Math.sqrt(d2), f = 1 - d / 161;
        x += dx / (d || 1) * f * 10; y += dy / (d || 1) * f * 10;
        a += f * .5; r += f * .7;
      }
      bucket[Math.min(BUCKETS - 1, Math.round(a * (BUCKETS - 1)))].push(x - r / 2, y - r / 2, r);
    }
    bucket.forEach((b, i) => {
      if (!b.length) return;
      ctx.fillStyle = `rgba(237,237,235,${(i / (BUCKETS - 1)).toFixed(3)})`;
      ctx.beginPath();
      for (let k = 0; k < b.length; k += 3) ctx.rect(b[k], b[k + 1], b[k + 2], b[k + 2]);
      ctx.fill();
    });
  }

  addEventListener("resize", () => { clearTimeout(build.t); build.t = setTimeout(() => { build(); lastE = null; dirty = true; }, 150); });
  addEventListener("pointermove", e => {
    const r = cv.getBoundingClientRect();
    mouse.tx = e.clientX - r.left; mouse.ty = e.clientY - r.top;
  }, { passive: true });
  new IntersectionObserver(([e]) => visible = e.isIntersecting).observe(cv);
  build();
  requestAnimationFrame(draw);
}

/* ───────────────────────── hero: the demo ───────────────────────── */
function heroDemo() {
  const stage = $("#hero-stage"), code = $("#hero-code"), term = $("#hero-term"), cap = $("#hero-caption");
  const steps = $$(".step", stage), ind = $(".step-ind", stage);
  renderCode(code, SRC["avg.claw"]);

  const STEPS = [
    { cmd: "claw run avg.claw", out: OUT.avgRun, fail: true,
      cap: `A contract, broken. <b>What, where, and with which values.</b> Exit code 101.`,
      mark() { lineEl(code, 10).classList.add("hl"); },
      after() { lineEl(code, 10).classList.remove("hl"); lineEl(code, 11).classList.add("bad"); stage.classList.add("trap"); } },
    { cmd: "claw why avg.claw", out: OUT.avgWhy, fail: true,
      cap: `Not an address and a stack. <b>Only the lines that led here.</b>`,
      mark() { stage.classList.add("trap"); },
      after() {
        code.classList.add("why");
        lineEl(code, 2).classList.add("keep", "bad");
        lineEl(code, 11).classList.add("keep", "bad");
        anno(code, 2, "need failed", true);
        anno(code, 11, "left = 0, xs = []");
      } },
    { cmd: "claw build --stats avg.claw", out: OUT.avgBuild, fail: false,
      cap: `What the compiler can prove, it removes. <b>The rest it checks, out loud.</b>`,
      mark() { stage.classList.remove("trap"); },
      after() { lineEl(code, 2).classList.add("hl"); anno(code, 2, "kept: not provable here"); } },
  ];

  let run = 0, cur = -1;
  function moveInd(i) {
    const b = steps[i];
    ind.style.width = b.offsetWidth + "px";
    ind.style.transform = `translateX(${b.offsetLeft - 3}px)`;
    steps.forEach((s, k) => { s.classList.toggle("on", k === i); s.classList.toggle("done", k < i); s.setAttribute("aria-selected", k === i); });
  }
  function staticTo(i) {
    // history up to step i-1, instantly
    clearMarks(code); stage.classList.remove("trap");
    let h = "";
    for (let k = 0; k < i; k++) {
      h += `<div class="tl cmdl">${promptHTML(STEPS[k].cmd, k > 0 && STEPS[k - 1].fail)}</div>` + STEPS[k].out.map(([c, t]) => fmtOut(t, c)).join("");
    }
    term.innerHTML = h;
    while (term.children.length > 11) term.removeChild(term.firstChild);
  }
  async function play(from) {
    const id = ++run;
    for (let i = from; i < STEPS.length; i++) {
      if (id !== run) return;
      cur = i; moveInd(i);
      staticTo(i);
      const s = STEPS[i];
      cap.innerHTML = `<span class="cap">${s.cap}</span>`;
      const line = document.createElement("div");
      line.className = "tl cmdl";
      line.innerHTML = promptHTML("", i > 0 && STEPS[i - 1].fail);
      term.appendChild(line);
      s.mark();
      await sleep(reduced ? 0 : 450);
      if (id !== run) return;
      await typeInto(line, s.cmd, reduced ? 0 : 34);
      await sleep(reduced ? 0 : 380);
      for (const [c, t] of s.out) {
        if (id !== run) return;
        term.insertAdjacentHTML("beforeend", fmtOut(t, c));
        while (term.children.length > 11) term.removeChild(term.firstChild);
        await sleep(reduced ? 0 : 110);
      }
      s.after();
      await sleep(reduced ? 0 : 3600);
    }
    if (id === run) {
      const done = document.createElement("div");
      done.className = "tl cmdl";
      done.innerHTML = promptHTML("", false) + '<span class="caret"></span>';
      term.appendChild(done);
    }
  }
  steps.forEach((b, i) => b.addEventListener("click", () => play(i)));
  $("#hero-replay").addEventListener("click", () => play(0));
  addEventListener("resize", () => cur >= 0 && moveInd(cur));

  let started = false;
  new IntersectionObserver(([e], o) => {
    if (e.isIntersecting && !started) { started = true; o.disconnect(); setTimeout(() => play(0), 900); }
  }, { threshold: .35 }).observe(stage);
}

/* ───────────────────────── manifesto: words light as you scroll ───────────────────────── */
function manifesto() {
  const el = $("#mani"); if (!el) return;
  const wrapWords = node => {
    [...node.childNodes].forEach(ch => {
      if (ch.nodeType === 3) {
        const frag = document.createDocumentFragment();
        ch.textContent.split(/(\s+)/).forEach(w => {
          if (!w) return;
          if (/^\s+$/.test(w)) frag.appendChild(document.createTextNode(w));
          else { const s = document.createElement("span"); s.className = "mw"; s.textContent = w; frag.appendChild(s); }
        });
        ch.replaceWith(frag);
      } else wrapWords(ch);
    });
  };
  wrapWords(el);
  const words = $$(".mw", el);
  const upd = () => {
    const r = el.getBoundingClientRect(), vh = innerHeight;
    const p = Math.min(1, Math.max(0, (vh * .85 - r.top) / (r.height + vh * .35)));
    const n = Math.round(p * words.length * 1.05);
    words.forEach((w, i) => w.classList.toggle("lit", i < n));
  };
  addEventListener("scroll", upd, { passive: true }); upd();
}

/* ───────────────────────── reveal, counters, spotlight, nav ───────────────────────── */
function reveals() {
  const io = new IntersectionObserver(es => es.forEach(e => {
    if (!e.isIntersecting) return;
    e.target.classList.add("in");
    $$("[data-count]", e.target).forEach(countUp);
    io.unobserve(e.target);
  }), { threshold: .14, rootMargin: "0px 0px -6% 0px" });
  $$(".reveal").forEach(el => io.observe(el));
  $$(".shrink span").forEach((s, i) => s.style.setProperty("--i", i));
}
function countUp(el) {
  if (el.dataset.done) return; el.dataset.done = 1;
  const to = parseFloat(el.dataset.count), dec = +(el.dataset.dec || 0);
  if (reduced || to === 0) return;
  const t0 = performance.now(), D = 1400;
  const f = now => {
    const p = Math.min(1, (now - t0) / D), e = 1 - Math.pow(1 - p, 4);
    el.textContent = (to * e).toLocaleString("en-US", { minimumFractionDigits: dec, maximumFractionDigits: dec });
    if (p < 1) requestAnimationFrame(f);
  };
  requestAnimationFrame(f);
}
function spotlight() {
  addEventListener("pointermove", e => {
    const g = e.target.closest?.(".glass"); if (!g) return;
    const r = g.getBoundingClientRect();
    g.style.setProperty("--mx", e.clientX - r.left + "px");
    g.style.setProperty("--my", e.clientY - r.top + "px");
  }, { passive: true });
}
function nav() {
  const n = $("#nav"), bar = $(".progress i");
  let last = scrollY;
  const links = $$(".nav-links a");
  const secs = links.map(a => $(a.getAttribute("href")));
  addEventListener("scroll", () => {
    const y = scrollY;
    if (y > 300 && y > last + 2) n.classList.add("away");
    else if (y < last - 2 || y < 300) n.classList.remove("away");
    last = y;
    bar.style.setProperty("--p", y / (document.documentElement.scrollHeight - innerHeight));
    let on = -1;
    secs.forEach((s, i) => { if (s && s.getBoundingClientRect().top < innerHeight * .4) on = i; });
    links.forEach((a, i) => a.classList.toggle("on", i === on));
  }, { passive: true });
}

/* ───────────────────────── copy + toast ───────────────────────── */
function toast(msg) {
  const t = $("#toast"); t.textContent = msg; t.classList.add("show");
  clearTimeout(toast.t); toast.t = setTimeout(() => t.classList.remove("show"), 1600);
}
async function copy(text) {
  try { await navigator.clipboard.writeText(text); toast("copied"); }
  catch { toast(text); }
}
function copies() {
  $$("[data-copy]").forEach(b => b.addEventListener("click", () => {
    copy(b.dataset.copy); b.classList.add("copied"); setTimeout(() => b.classList.remove("copied"), 1400);
  }));
}

/* ───────────────────────── 01: card, dots, keywords ───────────────────────── */
function cardSection() {
  const src = $("#card-src")?.textContent.trim() || "";
  $("#card-pre").innerHTML = esc(src).replace(/^(#+ .*)$/gm, "<b>$1</b>");
  $$(".dots").forEach(d => {
    const of = +d.dataset.of, ok = +d.dataset.ok;
    d.innerHTML = Array.from({ length: of }, (_, i) => `<i class="${i < ok ? "ok" : ""}" style="transition-delay:${i * 40 + 200}ms"></i>`).join("");
  });

  const KWS = [
    ["fn", "a function, safe by default"], ["type", "a struct, or variants with |"], ["use", "bring in a module"],
    ["pub", "export it; must declare its effects"], ["const", "computed while compiling"], ["mut", "this one may change"],
    ["own", "takes the value over"], ["if", "branch"], ["else", "the other branch, or a default for an error"],
    ["for", "the only loop: range, while, forever"], ["in", "walk a range or collection"], ["break", "leave the loop"],
    ["continue", "next turn of the loop"], ["match", "must cover every case"], ["ret", "return"],
    ["fail", "return an error value"], ["need", "what a function asks for, checked at every call"],
    ["sure", "what it promises, checked at every return"], ["test", "a test block, beside the code"],
    ["claw", "this function is plain C", 1], ["defer", "run when the block ends"], ["extern", "call any C function", 1],
    ["and", "no &&"], ["or", "no ||"], ["not", "no !"], ["true", ""], ["false", ""], ["none", "a missing value; null in claw fn"],
  ];
  const wall = $("#kw-wall"), hint = $("#kw-hint");
  wall.innerHTML = KWS.map(([k, , u]) => `<span class="${u ? "u" : ""}" tabindex="0">${k}</span>`).join(" ");
  const say = i => {
    const [k, d] = KWS[i];
    hint.innerHTML = `<b>${k}</b>${d || "a boolean"}`;
  };
  $$("span", wall).forEach((s, i) => {
    s.addEventListener("pointerenter", () => say(i));
    s.addEventListener("focus", () => say(i));
  });
  wall.addEventListener("pointerleave", () => hint.innerHTML = "<span>hover a word</span>");
  $("#foot-kw").innerHTML = KWS.map(([k]) => `<span>${k}</span>`).join(" ");
  // a slow tour through the keywords while nobody is pointing at them
  const spans = $$("span", wall);
  let tourI = 0, tourOn = false, hover = false;
  wall.addEventListener("pointerenter", () => { hover = true; spans.forEach(s => s.classList.remove("on")); });
  wall.addEventListener("pointerleave", () => hover = false);
  new IntersectionObserver(([e]) => tourOn = e.isIntersecting).observe(wall);
  if (!reduced) setInterval(() => {
    if (!tourOn || hover) return;
    spans.forEach(s => s.classList.remove("on"));
    spans[tourI].classList.add("on"); say(tourI);
    tourI = (tourI + 1) % spans.length;
  }, 1400);
}

/* ───────────────────────── 02: paw & claw ───────────────────────── */
function gears() {
  const sec = $("#gears"), code = $("#gear-code"), term = $("#gear-term");
  const btns = $$(".gs-btn", sec), thumb = $(".gs-thumb", sec);
  const MODES = {
    paw: { src: SRC["pets.claw"].split("\n\nfn total")[0] + `

fn main() {
    a := pets()
    b := copy(a)             // a copy is always explicit
    print("{len(a)} {len(b)}")
    print(join(a, " "))
}                            // a and b are freed here`,
      cmd: "claw run pets.claw", out: [["", "3 3"], ["", "cat dog bird"]] },
    claw: { src: SRC["paw_and_claw.claw"], cmd: "claw audit paw_and_claw.claw", out: OUT.audit },
  };
  let mode = null;
  const set = (m, first) => {
    if (m === mode) return; mode = m;
    sec.dataset.gear = m;
    btns.forEach(b => { const on = b.dataset.gear === m; b.classList.toggle("on", on); b.setAttribute("aria-selected", on); });
    const b = btns.find(b => b.dataset.gear === m);
    thumb.style.width = b.offsetWidth + "px"; thumb.style.transform = `translateX(${b.offsetLeft - 4}px)`;
    renderCode(code, MODES[m].src, { swap: !first });
    term.innerHTML = `<div class="tl cmdl">${promptHTML(MODES[m].cmd)}</div>` + MODES[m].out.map(([c, t]) => fmtOut(t, c)).join("");
  };
  btns.forEach(b => b.addEventListener("click", () => set(b.dataset.gear)));
  addEventListener("resize", () => { const m = mode; mode = null; set(m, true); });
  set("paw", true);
}

/* ───────────────────────── 03: playground ───────────────────────── */
function playground() {
  const FILES = ["avg.claw", "pick.claw", "move.claw", "pets.claw", "paw_and_claw.claw"];
  const st = { file: "pick.claw", moveFixed: false, pickNeed: false, last: false, hist: [], hi: 0 };
  const code = $("#play-code"), term = $("#play-term"), files = $("#play-files"), chipsEl = $("#play-chips");
  const form = $("#play-form"), input = $("#play-in");

  const srcOf = f => f === "move.claw" && st.moveFixed ? SRC["move.claw+fix"] : f === "pick.claw" && st.pickNeed ? SRC["pick.claw+need"] : SRC[f];
  files.innerHTML = FILES.map(f => `<button role="tab" data-f="${f}">${f}</button>`).join("");
  const open = (f, swap) => {
    st.file = f;
    $$("button", files).forEach(b => { b.classList.toggle("on", b.dataset.f === f); b.setAttribute("aria-selected", b.dataset.f === f); });
    renderCode(code, srcOf(f), { swap });
  };
  $$("button", files).forEach(b => b.addEventListener("click", () => open(b.dataset.f, true)));

  function chips() {
    const c = [];
    if (!st.pickNeed) c.push(["claw test pick.claw", 1]);
    else c.push(["claw test pick.claw", 1]);
    c.push(["claw why avg.claw"]);
    c.push([st.moveFixed ? "claw run move.claw" : "claw check move.claw"]);
    if (!st.moveFixed) c.push(["claw fix move.claw"]);
    c.push(["claw test pets.claw"], ["claw sig pets.claw"], ["claw audit paw_and_claw.claw"], ["claw build --stats avg.claw"], ["claw help"]);
    if (st.suggest) c.unshift([st.suggest, 2]);
    chipsEl.innerHTML = c.map(([t, k], i) => `<button class="chip ${k === 2 ? "edit" : i === 0 && st.next ? "next" : ""}" data-c="${esc(t)}">${esc(t)}</button>`).join("");
    $$(".chip", chipsEl).forEach(b => b.addEventListener("click", () => exec(b.dataset.c, true)));
  }

  function respond(raw) {
    const cmd = raw.trim().replace(/\s+/g, " ");
    const m = cmd.match(/^claw\s+(\S+)(?:\s+(.*))?$/);
    if (cmd === "clear") return { clear: true };
    if (cmd === "ls") return { out: FILES.map(f => ["", f]) };
    if (/^(cat|open|vim|nano|code) /.test(cmd)) {
      const f = cmd.split(" ")[1];
      return FILES.includes(f) ? { file: f, out: [["ui", `opened ${f} on the left`]] } : { out: [["ui", `no such file here: ${f}`]] };
    }
    if (cmd === "add the suggested need") {
      st.pickNeed = true; st.suggest = null;
      return { file: "pick.claw", edit: [2], out: [["ui", "You edited pick.claw: line 2 is now the suggested need. Run the test again."]] };
    }
    if (cmd === "help" || cmd === "claw help" || cmd === "claw") return { out: OUT.help };
    if (!m) return { out: [["ui", "This terminal only replays recorded claw output. Try one of the commands below."]] };
    const [, sub, rest = ""] = m;
    if (sub === "--version") return { out: [["", "claw 0.10.0"]] };
    const args = rest.split(" ").filter(Boolean);
    const f = args.find(a => a.endsWith(".claw"));
    const flags = args.filter(a => a.startsWith("--")).join(" ");
    if (sub === "card") return { out: $("#card-src").textContent.trim().split("\n").slice(0, 14).map(l => ["", l]).concat([["ui", "… the full card scrolls in section 01."]]) };
    if (!f) return { out: [["err", `claw: ${sub} needs a file, like \`claw ${sub} avg.claw\``]], fail: true };
    if (!FILES.includes(f)) return { out: [["err", `claw: cannot read ${f}`]], fail: true };
    const key = `${sub} ${f}`;
    switch (key) {
      case "run avg.claw": return { file: f, out: OUT.avgRun, fail: true, bad: [11] };
      case "why avg.claw": return { file: f, out: OUT.avgWhy, fail: true, why: { 2: ["need failed", 1], 11: ["left = 0, xs = []"] } };
      case "build avg.claw": return flags.includes("--stats") ? { file: f, out: OUT.avgBuild } : { file: f, out: [["ui", "built ./avg (signed). Add --stats to see which checks were proved away."]] };
      case "check avg.claw": case "fix avg.claw": return { file: f, out: sub === "fix" ? [["", "fixed 0"]] : [] };
      case "test pick.claw":
        if (st.pickNeed) return { file: f, out: OUT.pickTestOk, hl: [2] };
        st.suggest = "add the suggested need";
        return { file: f, out: OUT.pickTest, fail: true, bad: [2] };
      case "run pick.claw": return { file: f, out: OUT.pickRun, fail: true };
      case "fix pick.claw": return { file: f, out: [["", "fixed 0"]] };
      case "check pick.claw": return { file: f, out: [] };
      case "check move.claw": case "run move.claw": case "build move.claw":
        if (st.moveFixed) return { file: f, out: sub === "run" ? [["", "6"]] : [] };
        return { file: f, out: OUT.moveCheck, fail: true, bad: [4], why: { 3: ["moved here"] } };
      case "fix move.claw":
        if (st.moveFixed) return { file: f, out: [["", "fixed 0"]] };
        st.moveFixed = true;
        return { file: f, out: [["ok", "fixed 1"]], edit: [3] };
      case "run pets.claw": return { file: f, out: OUT.petsRun };
      case "sig pets.claw": return { file: f, out: OUT.petsSig };
      case "test pets.claw": return { file: f, out: OUT.petsTest, hl: [11] };
      case "check pets.claw": case "check paw_and_claw.claw": return { file: f, out: [] };
      case "audit paw_and_claw.claw": return { file: f, out: OUT.audit, bad: [4, 6, 16] };
      case "run paw_and_claw.claw": return { file: f, out: OUT.pawRun };
      case "audit avg.claw": case "audit pick.claw": case "audit move.claw": case "audit pets.claw":
        return { file: f, out: [["", "audit: 0 claw fn, 0 extern, 0 checks not in `claw build`; std has 17 claw fn, not listed"]] };
    }
    return { file: f, out: [["ui", `Not recorded for this page: claw ${sub} ${f}. Try one of the commands below.`]] };
  }

  let busy = false;
  async function exec(raw, typed) {
    if (busy || !raw.trim()) return;
    busy = true;
    st.hist.push(raw); st.hi = st.hist.length;
    input.value = "";
    const isEdit = raw === "add the suggested need";
    if (!isEdit) {
      const line = document.createElement("div");
      line.className = "tl cmdl";
      line.innerHTML = promptHTML("", st.last);
      term.appendChild(line);
      if (typed && !reduced) await typeInto(line, raw.trim(), 16); else line.innerHTML = promptHTML(raw.trim(), st.last);
    }
    const r = respond(raw);
    if (r.clear) { term.innerHTML = ""; busy = false; chips(); return; }
    if (r.file) {
      const edited = r.edit;
      if (r.file !== st.file || edited) open(r.file, r.file !== st.file);
      clearMarks(code);
      (r.bad || []).forEach(n => lineEl(code, n)?.classList.add("bad"));
      (r.hl || []).forEach(n => lineEl(code, n)?.classList.add("hl"));
      (edited || []).forEach(n => { const l = lineEl(code, n); l?.classList.add(r.file === "pick.claw" ? "new" : "morph", "hl"); });
      if (r.why) {
        code.classList.add("why");
        Object.entries(r.why).forEach(([n, [t, hot]]) => { lineEl(code, +n)?.classList.add("keep"); anno(code, +n, t, hot); });
        (r.bad || []).forEach(n => lineEl(code, n)?.classList.add("keep"));
      }
    }
    await sleep(reduced ? 0 : 160);
    if (!r.out.length) term.insertAdjacentHTML("beforeend", fmtOut("(no output, exit 0)", "ui"));
    for (const [c, t] of r.out) {
      term.insertAdjacentHTML("beforeend", fmtOut(t, c));
      term.scrollTop = term.scrollHeight;
      await sleep(reduced ? 0 : 55);
    }
    st.last = !!r.fail;
    st.next = false;
    term.scrollTop = term.scrollHeight;
    busy = false;
    chips();
  }

  form.addEventListener("submit", e => { e.preventDefault(); exec(input.value, false); });
  input.addEventListener("keydown", e => {
    if (e.key === "ArrowUp" && st.hi > 0) { st.hi--; input.value = st.hist[st.hi]; e.preventDefault(); }
    if (e.key === "ArrowDown") { st.hi = Math.min(st.hist.length, st.hi + 1); input.value = st.hist[st.hi] || ""; e.preventDefault(); }
    if (e.key === "Tab" && input.value) {
      const all = $$(".chip", chipsEl).map(c => c.dataset.c);
      const hit = all.find(c => c.startsWith(input.value));
      if (hit) { input.value = hit; e.preventDefault(); }
    }
  });

  open("pick.claw");
  term.innerHTML = fmtOut("Recorded output from claw 0.10. Type a command, or pick one below.", "ui");
  st.next = true;
  chips();
  window.__play = (c) => { $("#tools").scrollIntoView({ behavior: "smooth" }); setTimeout(() => exec(c, true), 700); };
}

/* ───────────────────────── 04: signatures, contract ───────────────────────── */
function truth() {
  const say = $("#sig-say");
  $$(".sig").forEach(s => {
    s.addEventListener("pointerenter", () => say.textContent = s.dataset.say);
    s.addEventListener("pointerleave", () => say.textContent = "$ claw sig pets.claw");
  });
  renderCode($("#contract-code"), SRC["avg.claw"].split("\n\nfn main")[0]);
  lineEl($("#contract-code"), 2).classList.add("hl");
  $("#contract-term").innerHTML = `<div class="tl cmdl">${promptHTML("claw build --stats avg.claw")}</div>` + fmtOut(OUT.avgBuild[0][1], "ok");
}

/* ───────────────────────── 06: pipeline ───────────────────────── */
function pipeline() {
  const STAGES = [
    ["lexer", "lex", "Source text into tokens."],
    ["parser", "ast", "Tokens into a syntax tree."],
    ["sema", "sem", "Types, ownership and moves. Most E#### errors are born here."],
    ["effects", "+fx", "Works out +mem and +io for every function."],
    ["lower", "low", "The tree, lowered into Claw’s own IR."],
    ["CIR", "cir", "The IR. An interpreter runs it for const, for fuzzing and for claw why."],
    ["opt", "opt", "SSA, inlining, constant folding, repeats removed, work hoisted out of loops, proven checks dropped."],
    ["regalloc", "reg", "Values into machine registers."],
    ["codegen", "isa", "Its own ARM64 and x86-64 encoders. No assembler."],
    ["Mach-O / ELF", "obj", "Writes the executable file itself. No linker."],
    ["sign", "sha", "Signs it. The SHA-256 is written here too."],
  ];
  const list = $("#pipe-list"), say = $("#pipe-say"), pipe = $("#pipe");
  list.innerHTML = STAGES.map(([n, a], i) => `<li class="${i === STAGES.length - 1 ? "end" : ""}" tabindex="0"><span class="node">${a}</span><span class="lbl">${n}</span></li>`).join("");
  const items = $$("li", list);
  let auto = 0, hovering = false, timer;
  const show = i => {
    items.forEach((li, k) => { li.classList.toggle("lit", k <= i); li.classList.toggle("on", k === i); });
    say.innerHTML = `<span class="k">${STAGES[i][0]}</span>${STAGES[i][2]}`;
    pipe.classList.toggle("signing", i === STAGES.length - 1);
  };
  items.forEach((li, i) => {
    li.addEventListener("pointerenter", () => { hovering = true; show(i); });
    li.addEventListener("focus", () => { hovering = true; show(i); });
    li.addEventListener("pointerleave", () => hovering = false);
  });
  const tick = () => { if (!hovering) { show(auto); auto = (auto + 1) % STAGES.length; } };
  new IntersectionObserver(([e]) => {
    clearInterval(timer);
    if (e.isIntersecting) { tick(); timer = setInterval(tick, reduced ? 4000 : 1500); }
  }, { threshold: .3 }).observe(pipe);
}

/* ───────────────────────── 07: numbers ───────────────────────── */
function numbers() {
  const B = [
    ["primes", "", .189, .189], ["sieve", "", .091, .098], ["matmul_simd", "SIMD by hand", .130, .144],
    ["mandel", "", .103, .139], ["collatz", "", .198, .286], ["fib(40)", "", .150, .289], ["matmul", "no SIMD", .129, .366],
  ];
  const RATIO = ["1.0", "1.1", "1.1", "1.4", "1.4", "1.9", "2.8"]; // as published in benchmarks/RESULTS.md
  const max = .4;
  $("#bench-rows").innerHTML = B.map(([n, s, c, w], i) => {
    const r = +RATIO[i];
    return `<div class="brow">
      <div class="brow-name">${n}${s ? `<small>${s}</small>` : ""}</div>
      <div class="brow-bars">
        <i class="c" style="--v:${c / max};--d:${i}"></i>
        <i class="w" style="--v:${w / max};--d:${i + .5}"></i>
        <span class="t" style="left:calc(${w / max * 100}% + 10px)">${c.toFixed(3)} s → ${w.toFixed(3)} s</span>
      </div>
      <div class="brow-r ${r < 1.15 ? "tie" : r > 1.8 ? "slow" : ""}">${RATIO[i]}×</div>
    </div>`;
  }).join("");

  const btn = $("#compile-btn"), bar = $("#cbar"), out = $("#compile-out");
  btn.addEventListener("click", async () => {
    bar.classList.remove("go"); void bar.offsetWidth;
    out.innerHTML = `<span class="dim">compiling 44,000 lines…</span>`;
    const t0 = performance.now();
    bar.classList.add("go");
    await sleep(137);
    const dt = Math.round(performance.now() - t0);
    out.innerHTML = `done in ${dt} ms · signed · ~${Math.round(44000 / dt * 1000 / 1000)}k lines/s <span class="dim">(replayed at the measured 137 ms)</span>`;
  });
}

/* ───────────────────────── 08: spec ───────────────────────── */
function spec() {
  const S = [
    ["Types", "<code>int</code> (64-bit), <code>i8 i16 i32 u8 u16 u32 u64</code>, <code>f32 f64</code>, <code>bool</code>, <code>str</code>, structs, variants (<code>type Shape = Circle{r f64} | Dot</code>), arrays <code>[N]T</code>, growable <code>[]T</code>, <code>?T</code> for a value that may be missing, <code>T ! E</code> for one that may fail."],
    ["Checked arithmetic", "Signed overflow, division by zero, out-of-range index and bad shifts stop with <code>file:line:col</code> and exit code 101. Unsigned types wrap, as in C."],
    ["Control", "<code>match</code> that must cover every case. Errors are values: <code>fail</code>, <code>?</code>, <code>else</code>. No exceptions, no null."],
    ["Memory", "Ownership with <code>own</code>, read-only by default, change through <code>&amp;</code>, explicit <code>copy</code>. No dangling references, no leaks, no races."],
    ["Claw mode", "Pointers, <code>alloc</code> / <code>free</code>, <code>defer</code>, any C function through <code>extern fn</code>, <code>cast</code>, <code>sizeof</code>."],
    ["Abstraction", "Templates (<code>fn max[T]</code>, <code>type Pair[A, B]</code>), modules with <code>use</code>, a hash map and string functions in the standard library, functions as values (<code>fn(int) int</code>)."],
    ["Compile time", "<code>const big int = twice(limit)</code> runs while the program builds."],
    ["Threads", "<code>par.spawn(work, x)</code> and <code>par.join(t)</code>. A task takes its input with <code>own</code> and hands back its result through <code>join</code>: no data races by construction."],
    ["SIMD", "<code>f64x2</code>, <code>f32x4</code>, <code>i32x4</code> with <code>vload</code>, <code>vstore</code>, <code>lane</code>, <code>fma</code>. NEON on arm64, SSE on x86-64."],
    ["Build modes", "<code>claw run</code>: every check, plus heap misuse and leaks. <code>claw build</code>: optimized, only unproven checks stay. <code>claw build --fast</code>: no checks, risk like C."],
    ["Tools", "<code>run build test why fix check fmt sig ctx audit card lsp</code>, and <code>-g</code> so lldb stops on source lines."],
    ["Optimizer", "SSA, inlining, constant folding, repeated work removed, invariants hoisted out of loops, register allocation, and checks the compiler proved unnecessary removed."],
    ["Targets", "macOS arm64, macOS x86-64, ARM64 Linux."],
  ];
  $("#spec-list").innerHTML = S.map(([t, d]) => `<div class="reveal"><dt>${t}</dt><dd>${d}</dd></div>`).join("");
}

/* ───────────────────────── command palette ───────────────────────── */
function palette() {
  const pal = $("#palette"), inp = $("#palette-in"), list = $("#palette-list");
  const go = id => () => $(id).scrollIntoView({ behavior: reduced ? "auto" : "smooth" });
  const ITEMS = [
    ["The question", "section", go("#question")],
    ["Fits in a prompt", "section", go("#prompt")],
    ["Paw & Claw", "section", go("#gears")],
    ["The compiler: playground", "section", go("#tools")],
    ["Signatures & contracts", "section", go("#truth")],
    ["Tokens", "section", go("#tokens")],
    ["Source to binary", "section", go("#metal")],
    ["Benchmarks", "section", go("#numbers")],
    ["Spec sheet", "section", go("#spec")],
    ["Honest edges", "section", go("#limits")],
    ["Get started", "section", go("#start")],
    ["claw test pick.claw", "run", () => window.__play("claw test pick.claw")],
    ["claw why avg.claw", "run", () => window.__play("claw why avg.claw")],
    ["claw fix move.claw", "run", () => window.__play("claw fix move.claw")],
    ["claw audit paw_and_claw.claw", "run", () => window.__play("claw audit paw_and_claw.claw")],
    ["Switch to Claw mode", "action", () => { go("#gears")(); setTimeout(() => $('.gs-btn[data-gear="claw"]').click(), 600); }],
    ["Copy install commands", "action", () => copy("cargo build --release --manifest-path compiler/Cargo.toml\npython3 tools/install.py\nclaw run examples/tour.claw")],
  ];
  let shown = [], sel = 0;
  const render = () => {
    const q = inp.value.toLowerCase().trim();
    shown = ITEMS.filter(([t, g]) => !q || (t + " " + g).toLowerCase().includes(q));
    sel = Math.min(sel, Math.max(0, shown.length - 1));
    list.innerHTML = shown.length ? shown.map(([t, g], i) => `<li role="option" class="${i === sel ? "on" : ""}" data-i="${i}"><span>${esc(t)}</span><span class="grp">${g}</span></li>`).join("")
      : `<li class="empty">Nothing matches.</li>`;
    $$("li[data-i]", list).forEach(li => {
      li.addEventListener("click", () => run(+li.dataset.i));
      li.addEventListener("pointermove", () => { if (sel !== +li.dataset.i) { sel = +li.dataset.i; render(); } });
    });
    list.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  };
  const openP = () => { pal.hidden = false; inp.value = ""; sel = 0; render(); setTimeout(() => inp.focus(), 10); };
  const close = () => { pal.hidden = true; };
  const run = i => { const it = shown[i]; close(); it && it[2](); };
  $("#open-palette").addEventListener("click", openP);
  pal.addEventListener("click", e => { if (e.target === pal) close(); });
  inp.addEventListener("input", () => { sel = 0; render(); });
  addEventListener("keydown", e => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); pal.hidden ? openP() : close(); return; }
    if (pal.hidden) {
      if (e.key === "/" && !/input|textarea/i.test(document.activeElement.tagName)) { e.preventDefault(); openP(); }
      return;
    }
    if (e.key === "Escape") close();
    if (e.key === "ArrowDown") { sel = (sel + 1) % shown.length; render(); e.preventDefault(); }
    if (e.key === "ArrowUp") { sel = (sel - 1 + shown.length) % shown.length; render(); e.preventDefault(); }
    if (e.key === "Enter") { run(sel); e.preventDefault(); }
  });
}

/* ───────────────────────── boot ───────────────────────── */
function boot() {
  $$(".hero-title .w").forEach((w, i) => w.style.setProperty("--i", i));
  spec();
  cardSection();
  reveals();
  heroCanvas();
  heroDemo();
  manifesto();
  gears();
  playground();
  truth();
  pipeline();
  numbers();
  palette();
  copies();
  spotlight();
  nav();
  requestAnimationFrame(() => document.body.classList.add("ready"));
}
boot();
})();
