/* Claw — the manual. Vanilla JS, no build step. */
(() => {
"use strict";

const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
const esc = s => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const sleep = ms => new Promise(r => setTimeout(r, ms));
const mqReduced = matchMedia("(prefers-reduced-motion: reduce)");
let reduced = mqReduced.matches;
mqReduced.addEventListener?.("change", e => reduced = e.matches);

/* ═════════════ highlighter ═════════════ */
const KW = new Set("fn type use pub const mut own if else for in break continue match ret fail test defer and or not true false none".split(" "));
const CONTRACT = new Set(["need", "sure"]);
const UNSAFE = new Set(["claw", "extern", "alloc", "free", "cast"]);
const TYPES = new Set("int i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 bool str f64x2 f32x4 i32x4".split(" "));

function hl(line) {
  let o = "", i = 0;
  const n = line.length;
  while (i < n) {
    const c = line[i], rest = line.slice(i);
    if (rest.startsWith("//")) { o += `<span class="c">${esc(rest)}</span>`; break; }
    if (c === '"') { let j = i + 1; while (j < n && line[j] !== '"') j++; o += `<span class="s">${esc(line.slice(i, j + 1))}</span>`; i = j + 1; continue; }
    if (c === "'") { const j = line.indexOf("'", i + 1); o += `<span class="s">${esc(line.slice(i, j + 1))}</span>`; i = j + 1; continue; }
    if (c === "+" && /^\+(mem|io)\b/.test(rest)) { const m = rest.match(/^\+(mem|io)/)[0]; o += `<span class="e">${m}</span>`; i += m.length; continue; }
    if (/[0-9]/.test(c)) { const m = rest.match(/^[0-9][0-9_]*(\.[0-9]+)?/)[0]; o += `<span class="n">${m}</span>`; i += m.length; continue; }
    if (/[A-Za-z_]/.test(c)) {
      const m = rest.match(/^[A-Za-z_][A-Za-z0-9_]*/)[0], after = line[i + m.length];
      let k = "";
      if (UNSAFE.has(m)) k = "u"; else if (CONTRACT.has(m)) k = "ct"; else if (KW.has(m)) k = "k";
      else if (TYPES.has(m) || /^[A-Z]/.test(m)) k = "t"; else if (after === "(" || after === "[") k = "f";
      o += k ? `<span class="${k}">${m}</span>` : m; i += m.length; continue;
    }
    if (c === "*" && /[A-Za-z(]/.test(line[i + 1] || "") && !/[A-Za-z0-9_)\]]/.test(line[i - 1] || "")) { o += `<span class="u">*</span>`; i++; continue; }
    if ("{}()[],:".includes(c)) { o += `<span class="p">${esc(c)}</span>`; i++; continue; }
    o += esc(c); i++;
  }
  return o;
}
function codeHTML(src) {
  return src.replace(/\n$/, "").split("\n").map((l, k) =>
    `<div class="ln" data-l="${k + 1}"><span class="no" aria-hidden="true">${k + 1}</span><span class="src">${hl(l) || " "}</span></div>`).join("");
}

/* ═════════════ the files (real) and their output (recorded from claw 0.10) ═════════════ */
const SRC = {
  "avg.claw": `fn avg(xs []int) int
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
  "pick.claw": `fn pick(xs []int, i int) int {
    ret xs[i]
}`,
  "pick.claw+need": `fn pick(xs []int, i int) int
    need i >= 0 and i < len(xs)
{
    ret xs[i]
}`,
  "move.claw": `fn main() {
    a []int = [1, 2, 3]
    b := a
    print(len(a) + len(b))
}`,
  "move.claw+fix": `fn main() {
    a []int = [1, 2, 3]
    b := copy(a)
    print(len(a) + len(b))
}`,
  "pets.claw": `fn pets() []str {
    mut a []str = []
    push(&a, "cat")
    push(&a, "dog")
    push(&a, "bird")
    ret a
}

fn total(xs []int) int {
    mut s := 0
    for x in xs { s += x }
    ret s
}

fn main() {
    a := pets()
    b := copy(a)
    print("{len(a)} {len(b)}")
    print(join(a, " "))
    print(total([1, 2, 3]))
}`,
  "paw_and_claw.claw": `// expect: 6
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
  "paw.claw": `fn pets() []str {
    mut a []str = []
    push(&a, "cat")
    push(&a, "dog")
    push(&a, "bird")
    ret a                // handed over, no copy
}

fn main() {
    a := pets()
    b := copy(a)         // a copy is always explicit
    print("{len(a)} {len(b)}")
    print(join(a, " "))
}                        // a and b are freed here`,
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
};

/* ═════════════ the proof engine: code on a sheet, marked in blue pencil ═════════════ */
function rng(seed) { return () => { seed |= 0; seed = seed + 0x6D2B79F5 | 0; let t = Math.imul(seed ^ seed >>> 15, 1 | seed); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; }
function smooth(pts) {
  // Catmull-Rom through points → cubic Béziers
  let d = `M${pts[0][0].toFixed(1)} ${pts[0][1].toFixed(1)}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[i - 1] || pts[i], p1 = pts[i], p2 = pts[i + 1], p3 = pts[i + 2] || p2;
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    d += ` C${c1[0].toFixed(1)} ${c1[1].toFixed(1)} ${c2[0].toFixed(1)} ${c2[1].toFixed(1)} ${p2[0].toFixed(1)} ${p2[1].toFixed(1)}`;
  }
  return d;
}

class Proof {
  constructor(host) {
    this.host = host;
    host.innerHTML = `<div class="code"><div class="code-wrap"><div class="lines"></div><svg class="marks" aria-hidden="true"></svg></div></div><div class="notes" aria-live="polite"></div>`;
    this.wrap = $(".code-wrap", host); this.lines = $(".lines", host); this.svg = $(".marks", host); this.notes = $(".notes", host);
    this.seed = 7; this.nNote = 0; this.lastNoteBottom = -1;
  }
  set(src) { this.src = src.split("\n"); this.lines.innerHTML = codeHTML(src); this.clear(); this.measure(); }
  measure() {
    const first = $(".src", this.lines);
    const probe = document.createElement("span");
    probe.textContent = "0".repeat(40); probe.style.visibility = "hidden"; probe.style.position = "absolute";
    // offset* (layout boxes) rather than getBoundingClientRect: the sheets are slightly rotated
    first.appendChild(probe); this.cw = probe.offsetWidth / 40; probe.remove();
    const ln = $(".ln", this.lines);
    this.x0 = ln.offsetLeft + first.offsetLeft;
    this.lh = ln.offsetHeight;
    this.top0 = ln.offsetTop;
    this.codeTop = this.host.querySelector(".code").offsetTop + parseFloat(getComputedStyle(this.host.querySelector(".code")).paddingTop);
    this.svg.setAttribute("viewBox", `0 0 ${this.wrap.offsetWidth} ${this.wrap.offsetHeight}`);
  }
  x(col) { return this.x0 + (col - 1) * this.cw; }
  yTop(l) { return this.top0 + (l - 1) * this.lh; }
  yMid(l) { return this.yTop(l) + this.lh / 2; }
  lineEnd(l) { return this.x((this.src[l - 1] || "").replace(/\s+$/, "").length + 1); }
  clear() {
    this.svg.innerHTML = ""; this.notes.innerHTML = ""; this.nNote = 0; this.lastNoteBottom = -1;
    this.host.classList.remove("dimmed");
    $$(".ln", this.lines).forEach(l => l.classList.remove("keep", "ins"));
  }
  path(d, delay = 0, dur = .7, extra = "") {
    const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
    p.setAttribute("d", d); p.setAttribute("pathLength", "1"); p.setAttribute("class", "draw " + extra);
    p.style.setProperty("--delay", delay + "s"); p.style.setProperty("--dur", dur + "s");
    this.svg.appendChild(p); return p;
  }
  text(x, y, t, delay = 0, cls = "") {
    const e = document.createElementNS("http://www.w3.org/2000/svg", "text");
    e.setAttribute("x", x); e.setAttribute("y", y); e.setAttribute("class", cls); e.textContent = t;
    e.style.setProperty("--delay", delay + "s"); this.svg.appendChild(e); return e;
  }
  circle(l, c0, c1, delay = 0) {
    const r = rng(this.seed++), xa = this.x(c0), xb = this.x(c1 + 1);
    const cx = (xa + xb) / 2, cy = this.yMid(l), rx = (xb - xa) / 2 + 10, ry = this.lh / 2 + 2;
    const pts = [], a0 = -2.5 + r() * .4, turns = 1.12, N = 30;
    for (let i = 0; i <= N; i++) {
      const a = a0 + (i / N) * Math.PI * 2 * turns, j = 1 + (r() - .5) * .06 + i / N * .07;
      pts.push([cx + Math.cos(a) * rx * j, cy + Math.sin(a) * ry * j]);
    }
    this.path(smooth(pts), delay, .75);
  }
  underline(l, c0, c1, delay = 0) {
    const r = rng(this.seed++), xa = this.x(c0) - 3, xb = this.x(c1 + 1) + 3, y = this.yTop(l) + this.lh - 3;
    const pts = [[xa, y + r() * 2], [xa + (xb - xa) * .33, y - 1.5 + r() * 2], [xa + (xb - xa) * .66, y + 1 + r() * 2], [xb, y - 1 + r() * 2]];
    this.path(smooth(pts), delay, .5);
    this.path(smooth(pts.map(([px, py], i) => [px + (i ? 4 : 8), py + 4])), delay + .25, .45);
  }
  strike(l, c0, c1, delay = 0) {
    const xa = this.x(c0) - 4, xb = this.x(c1 + 1) + 4, y = this.yMid(l);
    this.path(`M${xa} ${y + 3} C${xa + (xb - xa) * .4} ${y - 1}, ${xa + (xb - xa) * .7} ${y + 2}, ${xb} ${y - 3}`, delay, .4);
    const cx = (xa + xb) / 2, yb = this.yTop(l) + this.lh + 1;
    this.path(`M${cx - 5} ${yb + 5} L${cx} ${yb - 2} L${cx + 5} ${yb + 5}`, delay + .35, .3);
  }
  arrow(l1, c1, l2, c2, delay = 0) {
    const x1 = this.x(c1) - 8, y1 = this.yMid(l1), x2 = this.x(c2) - 8, y2 = this.yMid(l2);
    const bend = Math.max(this.x0 - 6, 6) - Math.min(x1, x2) ;
    const cxA = Math.min(x1, x2) + bend - 34;
    this.path(`M${x1} ${y1} C${cxA} ${y1 - 6}, ${cxA} ${y2 + 6}, ${x2} ${y2}`, delay, .8);
    this.path(`M${x2 - 9} ${y2 - 6} L${x2} ${y2} L${x2 - 10} ${y2 + 4}`, delay + .7, .25);
  }
  bracket(l0, l1, delay = 0) {
    const x = this.x0 - 10, ya = this.yTop(l0) + 4, yb = this.yTop(l1) + this.lh - 4;
    this.path(`M${x + 6} ${ya} L${x} ${ya + 2} L${x} ${yb - 2} L${x + 6} ${yb}`, delay, .6);
  }
  caret(l, col, delay = 0) {
    const x = this.x(col) + this.cw / 2, yb = this.yTop(l) + this.lh + 1;
    this.path(`M${x - 6} ${yb + 6} L${x} ${yb - 3} L${x + 6} ${yb + 6}`, delay, .35);
  }
  tick(l, delay = 0) {
    const x = Math.max(4, this.x0 - 30), y = this.yMid(l);
    this.path(`M${x} ${y} L${x + 5} ${y + 6} L${x + 16} ${y - 8}`, delay, .4);
  }
  note(l, html, delay = 0) {
    const n = ++this.nNote;
    const x = this.lineEnd(l) + 14, y = this.yMid(l);
    const c = document.createElementNS("http://www.w3.org/2000/svg", "ellipse");
    c.setAttribute("cx", x + 7); c.setAttribute("cy", y); c.setAttribute("rx", 8); c.setAttribute("ry", 8);
    c.setAttribute("pathLength", "1"); c.setAttribute("class", "draw"); c.style.setProperty("--delay", delay + "s"); c.style.setProperty("--dur", ".35s");
    this.svg.appendChild(c);
    this.text(x + 7, y + 3.5, n, delay + .1, "num").setAttribute("text-anchor", "middle");
    const el = document.createElement("p");
    el.className = "note"; el.innerHTML = `<b>${n}</b>${html}`; el.style.setProperty("--delay", delay + .15 + "s");
    this.notes.appendChild(el);
    let top = this.codeTop + this.yTop(l) + 2;
    if (top < this.lastNoteBottom + 6) top = this.lastNoteBottom + 6;
    el.style.top = top + "px";
    this.lastNoteBottom = top + el.offsetHeight;
    return el;
  }
  dim(keep) { this.host.classList.add("dimmed"); keep.forEach(n => $(`.ln[data-l="${n}"]`, this.lines)?.classList.add("keep")); }
  flash(n, cls = "ins") { const l = $(`.ln[data-l="${n}"]`, this.lines); if (l) { l.classList.remove(cls); void l.offsetWidth; l.classList.add(cls); } }
}

/* galley: the strip of real output under a sheet */
function galleyLine(g, text, cls = "") { const d = document.createElement("div"); d.className = "gl " + cls; d.textContent = text; g.appendChild(d); return d; }
async function galleyType(g, cmd, speed = 26) {
  const d = galleyLine(g, "", "cmd typing");
  if (reduced) { d.textContent = cmd; d.classList.remove("typing"); return; }
  for (let i = 1; i <= cmd.length; i++) { d.textContent = cmd.slice(0, i); await sleep(speed + Math.random() * speed * .6); }
  d.classList.remove("typing");
}
async function galleyOut(g, lines, gap = 70) { for (const [c, t] of lines) { galleyLine(g, t, c); if (!reduced) await sleep(gap); } }

/* ═════════════ cover: kinetic type + the hero proof ═════════════ */
function splitLetters(el) {
  const walk = node => {
    [...node.childNodes].forEach(ch => {
      if (ch.nodeType === 3) {
        const f = document.createDocumentFragment();
        [...ch.textContent].forEach(chr => {
          if (chr === " ") { f.appendChild(document.createTextNode(" ")); return; }
          const s = document.createElement("span"); s.className = "kl"; s.textContent = chr; f.appendChild(s);
        });
        ch.replaceWith(f);
      } else if (ch.nodeType === 1 && ch.tagName !== "svg") walk(ch);
    });
  };
  walk(el);
}
function kinetic() {
  const groups = [...$$(".kinetic"), ...$$(".kinetic-all")];
  groups.forEach(splitLetters);
  const letters = groups.flatMap(g => $$(".kl", g).map(l => ({ el: l, giant: !!g.closest(".giant"), w: 125, g: g.closest(".giant") ? 880 : 820, tw: 125, tg: 0 })));
  letters.forEach(L => L.tg = L.g);
  let mx = -9999, my = -9999, running = false;
  const step = () => {
    let moving = false;
    for (const L of letters) {
      const r = L.el.getBoundingClientRect();
      if (r.bottom < -50 || r.top > innerHeight + 50) continue;
      const cx = r.left + r.width / 2, cy = r.top + r.height / 2, R = L.giant ? 420 : 280;
      const d = Math.hypot(mx - cx, my - cy), f = Math.max(0, 1 - d / R), e = f * f * (3 - 2 * f);
      L.tw = 125 - 50 * e; L.tg = (L.giant ? 880 : 820) - 560 * e;
      L.w += (L.tw - L.w) * .16; L.g += (L.tg - L.g) * .16;
      if (Math.abs(L.tw - L.w) > .2 || Math.abs(L.tg - L.g) > .5) moving = true;
      L.el.style.setProperty("--w", L.w.toFixed(1)); L.el.style.setProperty("--g", L.g.toFixed(0));
    }
    if (moving) requestAnimationFrame(step); else running = false;
  };
  const kick = () => { if (!running && !reduced) { running = true; requestAnimationFrame(step); } };
  addEventListener("pointermove", e => { if (e.pointerType === "touch") return; mx = e.clientX; my = e.clientY; kick(); }, { passive: true });
  document.addEventListener("pointerleave", () => { mx = my = -9999; kick(); });
  addEventListener("scroll", kick, { passive: true });
}

function heroProof() {
  const host = $('[data-proof="hero"]'), g = $("#hero-galley"), cap = $("#hero-cap");
  const P = new Proof(host);
  P.set(SRC["avg.claw"]);
  let run = 0;
  async function play() {
    const id = ++run;
    P.clear(); g.innerHTML = ""; cap.textContent = "proof 1: run";
    await sleep(reduced ? 0 : 500); if (id !== run) return;
    await galleyType(g, "claw run avg.claw"); if (id !== run) return;
    await galleyOut(g, OUT.avgRun);
    P.underline(11, 11, 17, .1);
    P.note(11, "trap · exit 101", .7);
    await sleep(reduced ? 0 : 2600); if (id !== run) return;
    P.clear(); g.innerHTML = ""; cap.textContent = "proof 2: why";
    await galleyType(g, "claw why avg.claw"); if (id !== run) return;
    await galleyOut(g, OUT.avgWhy);
    P.dim([2, 11]);
    P.circle(11, 11, 17, .1);
    P.arrow(11, 11, 2, 10, .7);
    P.circle(2, 10, 20, 1.3);
    P.note(2, "need failed", 1.8);
    P.note(11, "left = 0, xs = []", 2.1);
    cap.textContent = "proof 2: why";
  }
  $("#hero-again").addEventListener("click", play);
  let started = false;
  new IntersectionObserver(([e], o) => { if (e.isIntersecting && !started) { started = true; o.disconnect(); setTimeout(play, reduced ? 0 : 900); } }, { threshold: .4 }).observe(host);
  addEventListener("resize", debounce(() => { P.measure(); if (started) play(); }, 300));
}

/* ═════════════ §01 + reveals ═════════════ */
function reveals() {
  const io = new IntersectionObserver(es => es.forEach(e => {
    if (!e.isIntersecting) return;
    e.target.classList.add("in");
    $$("[data-count]", e.target).forEach(countUp);
    io.unobserve(e.target);
  }), { threshold: .18, rootMargin: "0px 0px -8% 0px" });
  $$(".sec-head .title, .sec-head .intro, .q-aside, .bignum, .onepage-side, .specimen, .two-col, .sigs, .four, .tok-src, .compile, .fixed, .fine-t, .recipe, .toc, .colo-grid").forEach(el => el.classList.add("rv"));
  $$(".rv, .question, .exam, .dotplot, .tok, .nolist, .facts, .giant-wrap").forEach(el => io.observe(el));
}
function countUp(el) {
  if (el.dataset.done) return; el.dataset.done = 1;
  const to = parseFloat(el.dataset.count), dec = +(el.dataset.dec || 0);
  if (reduced || !to) return;
  const t0 = performance.now(), D = 1300;
  const f = now => { const p = Math.min(1, (now - t0) / D), e = 1 - Math.pow(1 - p, 4);
    el.textContent = (to * e).toLocaleString("en-US", { minimumFractionDigits: dec, maximumFractionDigits: dec }); if (p < 1) requestAnimationFrame(f); };
  requestAnimationFrame(f);
}

/* ═════════════ §02: the printed page with a loupe, tallies, keyword specimen ═════════════ */
function onePage() {
  const src = $("#card-src").textContent.trim();
  const html = esc(src).replace(/^(#+ .*)$/gm, "<b>$1</b>");
  $("#card-pre").innerHTML = html; $("#card-pre-2").innerHTML = html;
  const page = $("#page"), inner = $("#page-inner"), loupe = $("#loupe"), lin = $(".loupe-in", loupe);
  const Z = 2.7;
  const sync = () => {
    const cs = getComputedStyle(inner);
    lin.style.width = inner.clientWidth + "px";
    lin.style.padding = `${cs.paddingTop} ${cs.paddingRight} ${cs.paddingBottom} ${cs.paddingLeft}`;
  };
  sync(); addEventListener("resize", debounce(sync, 200));
  page.addEventListener("pointermove", e => {
    if (e.pointerType === "touch") return;
    const r = inner.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top;
    if (y > r.height) { page.classList.remove("lens"); return; }
    const R = loupe.offsetWidth / 2;
    page.classList.add("lens");
    loupe.style.left = x - R + "px"; loupe.style.top = y - R + "px";
    lin.style.transform = `translate(${R - x * Z}px, ${R - y * Z}px) scale(${Z})`;
  });
  page.addEventListener("pointerleave", () => page.classList.remove("lens"));

  // tallies: groups of five, drawn in pencil
  $$(".tally").forEach(svg => {
    const of = +svg.dataset.of, ok = +svg.dataset.ok, r = rng(of * 31 + ok);
    let x = 4, d = "";
    for (let i = 0; i < of; i++) {
      const inGroup = i % 5, miss = i >= ok;
      if (inGroup === 4) {
        const gx = x - 4 * 8 - 3;
        d += `<path class="${miss ? "miss" : ""}" style="--i:${i}" pathLength="1" d="M${gx} ${26 + r() * 2} L${x + 2} ${8 + r() * 2}"/>`;
        x += 12;
      } else {
        d += `<path class="${miss ? "miss" : ""}" style="--i:${i}" pathLength="1" d="M${x + r() * 1.5} ${5 + r() * 2} L${x + 1 + r() * 1.5} ${30 + r() * 2}"/>`;
        x += 8;
      }
    }
    svg.setAttribute("viewBox", `0 0 ${x + 6} 36`); svg.style.maxWidth = (x + 6) * 1.3 + "px";
    svg.innerHTML = d;
  });

  const KWS = [
    ["fn", "a function, safe by default"], ["type", "a struct, or variants joined by |"], ["use", "bring in a module"], ["pub", "export it; must declare its effects"],
    ["const", "computed while compiling"], ["mut", "this one may change"], ["own", "takes the value over"], ["if", "a branch"], ["else", "the other branch, or a default for an error"],
    ["for", "the only loop: range, while, forever"], ["in", "walk a range or a collection"], ["break", "leave the loop"], ["continue", "next turn of the loop"],
    ["match", "must cover every case"], ["ret", "return"], ["fail", "return an error value"], ["need", "what a function asks for, checked at every call"],
    ["sure", "what it promises, checked at every return"], ["test", "a test block, beside the code"], ["claw", "this function is plain C", 1], ["defer", "run when the block ends"],
    ["extern", "call any C function", 1], ["and", "instead of &&"], ["or", "instead of ||"], ["not", "instead of !"], ["true", "a bool"], ["false", "a bool"], ["none", "a missing value; null in claw fn"],
  ];
  const grid = $("#kw-grid"), say = $("#kw-say"), base = say.innerHTML;
  grid.innerHTML = KWS.map(([k, d, u], i) => `<li class="${u ? "u" : ""}" tabindex="0" aria-label="${k}: ${esc(d)}"><span class="kw-n">${String(i + 1).padStart(2, "0")}</span><span class="kw-d">${esc(d)}</span><span class="kw-w">${k}</span></li>`).join("");
  const cells = $$("li", grid);
  const show = i => { say.innerHTML = `<b>${KWS[i][0]}</b>${esc(KWS[i][1])}${KWS[i][2] ? " · marked in pencil: the compiler trusts you here" : ""}`; };
  cells.forEach((c, i) => { c.addEventListener("pointerenter", () => show(i)); c.addEventListener("focus", () => show(i)); });
  grid.addEventListener("pointerleave", () => say.innerHTML = base);
  $("#colo-kw").textContent = KWS.map(k => k[0]).join("  ");
}

/* ═════════════ §03: Paw | Claw comparison ═════════════ */
function compare() {
  const box = $("#compare"), handle = $("#cmp-handle");
  $('[data-code="paw"]').innerHTML = `<div>${codeHTML(SRC["paw.claw"])}</div>`;
  $('[data-code="claw"]').innerHTML = `<div>${codeHTML(SRC["paw_and_claw.claw"])}</div>`;
  const out = (el, cmd, lines) => { el.innerHTML = ""; galleyLine(el, cmd, "cmd"); lines.forEach(([c, t]) => galleyLine(el, t, c)); };
  out($("#paw-out"), "claw run paw.claw", [["", "3 3"], ["", "cat dog bird"]]);
  out($("#claw-out"), "claw audit paw_and_claw.claw", OUT.audit);
  let v = 50;
  const set = p => { v = Math.max(4, Math.min(96, p)); box.style.setProperty("--split", v + "%"); handle.setAttribute("aria-valuenow", Math.round(v)); };
  const fromX = x => { const r = box.getBoundingClientRect(); set((x - r.left) / r.width * 100); };
  let drag = false;
  handle.addEventListener("pointerdown", e => { drag = true; handle.setPointerCapture(e.pointerId); handle.classList.add("drag"); });
  handle.addEventListener("pointermove", e => drag && fromX(e.clientX));
  const end = () => { drag = false; handle.classList.remove("drag"); };
  handle.addEventListener("pointerup", end); handle.addEventListener("pointercancel", end);
  box.addEventListener("click", e => { if (!e.target.closest(".cmp-handle")) fromX(e.clientX); });
  handle.addEventListener("keydown", e => {
    const k = { ArrowLeft: -5, ArrowRight: 5, Home: -100, End: 100 }[e.key];
    if (k !== undefined) { set(v + k); e.preventDefault(); }
  });
  // a gentle hint that it moves, once, when it first comes into view
  new IntersectionObserver(([e], o) => {
    if (!e.isIntersecting || reduced) return; o.disconnect();
    const t0 = performance.now();
    const f = now => { const p = Math.min(1, (now - t0) / 1600); set(50 + Math.sin(p * Math.PI * 2) * 14 * (1 - p)); if (p < 1 && !drag) requestAnimationFrame(f); };
    requestAnimationFrame(f);
  }, { threshold: .5 }).observe(box);
  $("#night-btn").addEventListener("click", e => setMode(document.documentElement.dataset.mode === "claw" ? "paw" : "claw", e));
}

/* ═════════════ §04: the desk ═════════════ */
function desk() {
  const FILES = ["avg.claw", "pick.claw", "move.claw", "pets.claw", "paw_and_claw.claw"];
  const host = $('[data-proof="desk"]'), g = $("#desk-galley"), tabs = $("#desk-files"), cmds = $("#desk-cmds"), acts = $("#desk-actions"), cap = $("#desk-cap");
  const P = new Proof(host);
  const st = { file: "pick.claw", moveFixed: false, pickNeed: false, cmd: null };
  const srcOf = f => f === "move.claw" && st.moveFixed ? SRC["move.claw+fix"] : f === "pick.claw" && st.pickNeed ? SRC["pick.claw+need"] : SRC[f];

  tabs.innerHTML = FILES.map(f => `<button role="tab" data-f="${f}" aria-selected="false">${f}</button>`).join("");
  const open = f => {
    st.file = f; P.set(srcOf(f));
    $$("button", tabs).forEach(b => b.setAttribute("aria-selected", b.dataset.f === f));
  };
  $$("button", tabs).forEach(b => b.addEventListener("click", () => { open(b.dataset.f); g.innerHTML = ""; acts.innerHTML = ""; cap.textContent = ""; markCmd(null); }));

  const LIST = [
    "claw test pick.claw", "claw why avg.claw", "claw check move.claw", "claw fix move.claw",
    "claw audit paw_and_claw.claw", "claw sig pets.claw", "claw test pets.claw", "claw build --stats avg.claw", "claw run avg.claw",
  ];
  cmds.innerHTML = LIST.map(c => `<li><button data-c="${c}">${c}</button></li>`).join("");
  $$("button", cmds).forEach(b => b.addEventListener("click", () => exec(b.dataset.c)));
  const markCmd = c => $$("button", cmds).forEach(b => b.classList.toggle("on", b.dataset.c === c));

  // each command: which file, what the galley says (real), and how the pencil marks it
  const PLAN = {
    "run avg.claw": { f: "avg.claw", out: () => OUT.avgRun, mark: () => { P.underline(11, 11, 17, .1); P.note(11, "trap · exit 101 · file:line:col", .6); } },
    "why avg.claw": { f: "avg.claw", out: () => OUT.avgWhy, mark: () => { P.dim([2, 11]); P.circle(11, 11, 17, .1); P.arrow(11, 11, 2, 10, .6); P.circle(2, 10, 20, 1.2); P.note(2, "need failed", 1.6); P.note(11, "left = 0, xs = []", 1.9); } },
    "build avg.claw": { f: "avg.claw", out: () => OUT.avgBuild, mark: () => { P.note(1, "2 of 4 checks proved away, 4 calls inlined", .2); P.underline(2, 5, 20, .4); P.note(2, "the rest stay, and run", .8); } },
    "test pick.claw": { f: "pick.claw", out: () => st.pickNeed ? OUT.pickTestOk : OUT.pickTest, mark: () => {
      if (st.pickNeed) { P.underline(2, 5, 31, .1); P.tick(4, .4); P.note(4, "4000 generated inputs, 0 failed", .6); return; }
      P.circle(2, 9, 13, .1); P.note(2, "nobody wrote this test: pick([], 0) → index 0, len 0", .6);
      P.caret(1, 29, 1.1); P.note(1, "insert here: need i >= 0 and i < len(xs)", 1.4);
      action("Accept the suggested need", () => { st.pickNeed = true; open("pick.claw"); P.flash(2); galleyLine(g, "You added line 2. Run the test again.", "ui"); acts.innerHTML = ""; next("claw test pick.claw"); });
    } },
    "run pick.claw": { f: "pick.claw", out: () => OUT.pickRun, mark: () => { P.note(1, "a library file: no main", .2); } },
    "check move.claw": { f: "move.claw", out: () => st.moveFixed ? [] : OUT.moveCheck, mark: () => {
      if (st.moveFixed) { P.tick(3, .1); P.note(3, "no errors", .3); return; }
      P.circle(4, 15, 15, .1); P.underline(3, 10, 10, .5); P.arrow(4, 15, 3, 10, .8); P.note(3, "moved away here", 1.3); P.note(4, "E0151 · fix: copy(a)", 1.5);
      next("claw fix move.claw");
    } },
    "fix move.claw": { f: "move.claw", out: () => st.moveFixed ? [["", "fixed 0"]] : [["ok", "fixed 1"]], mark: async () => {
      if (st.moveFixed) { P.note(3, "nothing left to fix", .2); return; }
      P.strike(3, 10, 10, .1); P.note(3, "→ copy(a)", .5);
      await sleep(reduced ? 0 : 1300);
      st.moveFixed = true; open("move.claw"); P.flash(3); P.underline(3, 10, 16, .1); P.note(3, "patched by claw fix", .4);
      next("claw check move.claw");
    } },
    "run move.claw": { f: "move.claw", out: () => st.moveFixed ? [["", "6"]] : OUT.moveCheck, mark: () => {} },
    "audit paw_and_claw.claw": { f: "paw_and_claw.claw", out: () => OUT.audit, mark: () => {
      P.bracket(4, 4, .1); P.note(4, "extern: effects taken on trust", .3);
      P.bracket(6, 14, .6); P.note(6, "claw fn: alloc free index arith deref", .8);
      P.bracket(16, 18, 1.1); P.note(16, "claw fn: extern", 1.3);
    } },
    "run paw_and_claw.claw": { f: "paw_and_claw.claw", out: () => OUT.pawRun, mark: () => {} },
    "sig pets.claw": { f: "pets.claw", out: () => OUT.petsSig, mark: () => {
      P.underline(1, 1, 16, .1); P.note(1, "+mem", .3); P.underline(9, 1, 22, .5); P.note(9, "pure", .7); P.underline(15, 1, 9, .9); P.note(15, "+mem +io", 1.1);
    } },
    "test pets.claw": { f: "pets.claw", out: () => OUT.petsTest, mark: () => { P.circle(11, 19, 24, .1); P.note(11, "overflows for total([4611686018427387903, 2, …]): add a need if that input is not meant to work", .6); } },
    "run pets.claw": { f: "pets.claw", out: () => OUT.petsRun, mark: () => {} },
  };
  function action(label, fn) { const b = document.createElement("button"); b.className = "act"; b.textContent = label; b.addEventListener("click", fn); acts.appendChild(b); }
  function next(c) { $$("button", cmds).forEach(b => b.classList.toggle("next", b.dataset.c === c)); }

  let busy = false;
  async function exec(raw) {
    const cmd = raw.trim().replace(/\s+/g, " ");
    if (!cmd || busy) return;
    busy = true; acts.innerHTML = ""; g.innerHTML = ""; markCmd(cmd); next(null);
    const m = cmd.match(/^claw\s+(\w+)(?:\s+(.*))?$/);
    const file = m && (m[2] || "").split(" ").find(a => a.endsWith(".claw"));
    const plan = m && file && PLAN[`${m[1]} ${file}`];
    if (plan && plan.f !== st.file) open(plan.f); else P.clear();
    cap.textContent = plan ? `proof · ${m[1]}` : "";
    await galleyType(g, cmd, 18);
    if (!plan) {
      if (cmd === "claw --version") galleyLine(g, "claw 0.10.0");
      else if (cmd === "help" || cmd === "claw" || cmd === "claw help") galleyLine(g, "This desk replays recorded output. Pick one of the commands on the left.", "ui");
      else galleyLine(g, "Not recorded for this page. Pick one of the commands on the left; the output of those is real.", "ui");
      busy = false; return;
    }
    const out = plan.out();
    if (!out.length) galleyLine(g, "(no output, exit 0)", "ui"); else await galleyOut(g, out, 45);
    await plan.mark();
    busy = false;
  }
  $("#desk-form").addEventListener("submit", e => { e.preventDefault(); const i = $("#desk-in"); exec(i.value); i.value = ""; });
  open("pick.claw");
  galleyLine(g, "Pick a command on the left. The galley shows what the compiler printed; the pencil shows what it means.", "ui");
  next("claw test pick.claw");
  window.__desk = c => { $("#desk").scrollIntoView({ behavior: reduced ? "auto" : "smooth" }); setTimeout(() => exec(c), reduced ? 0 : 700); };
  addEventListener("resize", debounce(() => { P.measure(); }, 250));
}

/* ═════════════ §06: dot plot, compile ═════════════ */
function numbers() {
  const B = [["primes", "", .189, .189, "1.0"], ["sieve", "", .091, .098, "1.1"], ["matmul_simd", "SIMD by hand", .130, .144, "1.1"], ["mandel", "", .103, .139, "1.4"],
    ["collatz", "", .198, .286, "1.4"], ["fib(40)", "", .150, .289, "1.9"], ["matmul", "no SIMD", .129, .366, "2.8"]];
  $("#dp-rows").innerHTML = B.map(([n, s, c, w, r], i) => {
    const x = (parseFloat(r) - 1) / 2;
    return `<li class="dp-row ${r === "1.0" ? "tie" : ""}" tabindex="0" aria-label="${n}: ${r} times clang, ${c} s versus ${w} s">
      <span class="dp-name">${n}${s ? `<small>${s}</small>` : ""}</span>
      <span class="dp-track" style="--x:${x};--i:${i}"><i class="dp-line"></i><i class="dp-dot"></i><span class="dp-secs">clang ${c.toFixed(3)} s · claw ${w.toFixed(3)} s</span></span>
      <span class="dp-v">${r}×</span></li>`;
  }).join("");
  const btn = $("#compile-btn"), bar = $("#crule"), out = $("#compile-out");
  btn.addEventListener("click", async () => {
    btn.disabled = true; bar.classList.remove("go"); void bar.offsetWidth;
    out.textContent = "compiling 44,000 lines…";
    const t0 = performance.now(); bar.classList.add("go"); await sleep(137);
    out.textContent = `done in ${Math.round(performance.now() - t0)} ms · signed · replayed at the measured 137 ms`;
    btn.disabled = false;
  });
}

/* ═════════════ §07: one function through the compiler (real artefacts) ═════════════ */
function metal() {
  const tok = (t, k) => `<span class="${k === "keyword" ? "kw" : ""}">${esc(t)}<small>${k}</small></span>`;
  const TOKS = [["fn", "keyword"], ["add", "name"], ["(", "punct"], ["a", "name"], ["int", "type"], [",", "punct"], ["b", "name"], ["int", "type"], [")", "punct"], ["int", "type"], ["{", "punct"],
    ["ret", "keyword"], ["a", "name"], ["+", "op"], ["b", "name"], ["}", "punct"]];
  const ASM = [["a9bf7bfd", "stp  x29, x30, [sp, #-0x10]!"], ["910003fd", "mov  x29, sp"], ["d100c3ff", "sub  sp, sp, #0x30"], ["f90003e0", "str  x0, [sp]"], ["f90007e1", "str  x1, [sp, #0x8]"],
    ["f94003e9", "ldr  x9, [sp]"], ["f9000be9", "str  x9, [sp, #0x10]"], ["f94007e9", "ldr  x9, [sp, #0x8]"], ["f9000fe9", "str  x9, [sp, #0x18]"], ["f9400be9", "ldr  x9, [sp, #0x10]"],
    ["f9400fea", "ldr  x10, [sp, #0x18]"], ["ab0a012b", "adds x11, x9, x10", 1], ["540000e6", "b.vs <trap>", 1], ["f90013eb", "str  x11, [sp, #0x20]"], ["f94013e0", "ldr  x0, [sp, #0x20]"],
    ["910003bf", "mov  sp, x29"], ["a8c17bfd", "ldp  x29, x30, [sp], #0x10"], ["d65f03c0", "ret"]];
  const S = [
    { t: "Source", d: "Three lines that add two numbers, and a main that prints the sum.", cap: ["add.claw", "source"],
      body: `<div class="code">${codeHTML("fn add(a int, b int) int {\n    ret a + b\n}\n\nfn main() {\n    print(add(2, 3))\n}")}</div>` },
    { t: "Lexer", d: "Source text into tokens. Keywords are plain words, each one token for a model too.", cap: ["lexer", "16 tokens"],
      body: `<div class="tokens">${TOKS.map(([t, k], i) => tok(t, k).replace("<span", `<span style="--i:${i}"`)).join("")}</div>` },
    { t: "Parser & sema", d: "A tree, then types, ownership and moves. Most E#### errors are born here.", cap: ["sema", "typed tree"],
      body: `<pre class="tree">fn add(a int, b int) int
└─ ret                              <span class="hl">int</span>
   └─ +    signed 64-bit, <span class="hl">checked</span>     <span class="hl">int</span>
      ├─ a                          <span class="hl">int</span>
      └─ b                          <span class="hl">int</span></pre>` },
    { t: "Effects", d: "The compiler works out +mem and +io. No plus is a promise.", cap: ["claw sig add.claw", "effects"],
      body: `<div class="em">fn add(a int, b int) int</div><div>fn main() <span class="hl">+mem +io</span></div>
        <p class="plate-note">add is pure, so claw test may call it with anything. It did, and found:<br><span class="em">note: add overflows for add(1, 9223372036854775807); add a \`need\` if that input is not meant to work</span></p>` },
    { t: "CIR", d: "Claw’s own IR. An interpreter runs it too: for const, for fuzzing, for claw why.", cap: ["claw build --no-opt --emit-cir", "fn add"],
      body: `<pre class="tree">fn add params=2 vregs=5 objects=[]
  b0:
    Mov { d: 2, s: 0 }
    Mov { d: 3, s: 1 }
    Bin { op: Add, d: 4, a: 2, b: 3,
          num: Num { bits: 64, signed: true, <span class="hl">checked: true</span> },
          <span class="hl">loc: Loc { line: 2, col: 9, file: 0 }</span> }
    Ret(Some(4))
  b1:
    Unreachable</pre><p class="plate-note">The trap already knows its address in your source: line 2, column 9.</p>` },
    { t: "Optimizer", d: "SSA, inlining, folding. With it on, add(2, 3) is computed while compiling.", cap: ["claw build --stats --emit-cir", "fn main"],
      body: `<pre class="tree">fn main params=0 vregs=8 objects=[]
  b0:
    <span class="hl">Const { d: 7, v: 5 }</span>
    BufInt { s: 7, signed: true }
    BufStr { text: "\\n" }
    Flush { fd: 1 }
    Ret(None)</pre><p class="plate-note">checks: 1 of 1 removed, 1 calls inlined. The function add is gone; only its answer is left.</p>` },
    { t: "Codegen", d: "Its own ARM64 encoder, checked against llvm-mc. No assembler.", cap: ["_add", "arm64, --no-opt"],
      body: `<div class="hex">${ASM.map(([h, a, x]) => `<span class="h">${h}</span><span class="${x ? "hl" : ""}">${esc(a)}</span>`).join("")}</div>
        <p class="plate-note">adds + b.vs: the overflow check costs two instructions.</p>` },
    { t: "Mach-O & signature", d: "It writes the executable and signs it itself. No linker. Even SHA-256 is written here.", cap: ["./add", "signed"],
      body: `<div class="hex"><span class="h">00000000</span><span><span class="hl">cffa edfe</span> 0c00 0001 0000 0000 0200 0000</span><span class="h">00000010</span><span>0f00 0000 e003 0000 8500 2000 0000 0000</span></div>
        <p class="plate-note">CDHash 4c3e3a1fb2f2ca7d506af3b9863259c543a0f212 · sha256</p>
        <div class="tree" style="margin-top:18px">$ ./add<br><span class="em">5</span></div>` },
  ];
  const scrolly = $("#scrolly"), stages = $("#stages"), plate = $("#plate");
  scrolly.style.setProperty("--n", S.length);
  stages.innerHTML = S.map((s, i) => `<li><span class="sn">${String(i + 1).padStart(2, "0")}</span><span class="st">${s.t}</span><span class="sd">${s.d}</span></li>`).join("");
  const items = $$("li", stages);
  const plateHTML = s => `<div class="plate-cap"><span>${esc(s.cap[0])}</span><span class="dim">${esc(s.cap[1])}</span></div><div class="plate-body">${s.body}</div>`;
  let cur = -1;
  const show = i => {
    if (i === cur) return; cur = i;
    items.forEach((li, k) => { li.classList.toggle("on", k === i); li.classList.toggle("done", k < i); });
    plate.innerHTML = plateHTML(S[i]);
  };
  const flat = matchMedia("(max-width: 900px)");
  const layout = () => {
    if (flat.matches || reduced) {
      scrolly.classList.add("flat");
      plate.style.display = "none";
      let list = $(".plate-list", scrolly);
      if (!list) { list = document.createElement("div"); list.className = "plate-list"; $(".scrolly-grid", scrolly).appendChild(list); }
      list.innerHTML = S.map((s, i) => `<div class="plate"><div class="plate-cap"><span>${String(i + 1).padStart(2, "0")} · ${esc(s.t)}</span><span class="dim">${esc(s.cap[0])}</span></div><div class="plate-body">${s.body}<p class="plate-note" style="color:var(--ink-2)">${esc(s.d)}</p></div></div>`).join("");
    } else {
      scrolly.classList.remove("flat"); plate.style.display = ""; $(".plate-list", scrolly)?.remove(); cur = -1; onScroll();
    }
  };
  const onScroll = () => {
    if (scrolly.classList.contains("flat")) return;
    const r = scrolly.getBoundingClientRect(), total = r.height - (innerHeight - 56);
    const p = Math.min(.999, Math.max(0, -r.top / total));
    show(Math.floor(p * S.length));
  };
  addEventListener("scroll", onScroll, { passive: true });
  flat.addEventListener?.("change", layout);
  layout();
}

/* ═════════════ §08 index ═════════════ */
function index() {
  const I = [
    ["Types", "<code>int</code> (64-bit), <code>i8 i16 i32 u8 u16 u32 u64</code>, <code>f32 f64</code>, <code>bool</code>, <code>str</code>, structs, variants, arrays <code>[N]T</code>, growable <code>[]T</code>, <code>?T</code> for a value that may be missing, <code>T ! E</code> for one that may fail."],
    ["Checked math", "Signed overflow, division by zero, out-of-range index and bad shifts stop with <code>file:line:col</code> and exit code 101. Unsigned types wrap, as in C."],
    ["Control", "<code>match</code> that must cover every case. Errors are values: <code>fail</code>, <code>?</code>, <code>else</code>. No exceptions, no null."],
    ["Memory", "Ownership with <code>own</code>, read-only by default, change through <code>&amp;</code>, explicit <code>copy</code>. No dangling references, no leaks, no races."],
    ["Claw mode", "Pointers, <code>alloc</code> / <code>free</code>, <code>defer</code>, any C function through <code>extern fn</code>, <code>cast</code>, <code>sizeof</code>."],
    ["Abstraction", "Templates (<code>fn max[T]</code>, <code>type Pair[A, B]</code>), modules with <code>use</code>, a hash map and string functions in the standard library, functions as values."],
    ["Compile time", "<code>const big int = twice(limit)</code> runs while the program builds."],
    ["Threads", "<code>par.spawn(work, x)</code> and <code>par.join(t)</code>. A task takes its input with <code>own</code> and hands back its result: no data races by construction."],
    ["SIMD", "<code>f64x2</code>, <code>f32x4</code>, <code>i32x4</code> with <code>vload</code>, <code>vstore</code>, <code>lane</code>, <code>fma</code>. NEON on arm64, SSE on x86-64."],
    ["Build modes", "<code>claw run</code>: every check, plus heap misuse and leaks. <code>claw build</code>: optimized, unproven checks stay. <code>--fast</code>: no checks, risk like C."],
    ["Tools", "<code>run build test why fix check fmt sig ctx audit card lsp</code>, and <code>-g</code> so lldb stops on source lines."],
    ["Optimizer", "SSA, inlining, constant folding, repeated work removed, invariants hoisted out of loops, register allocation, proven checks removed."],
    ["Targets", "macOS arm64, macOS x86-64, ARM64 Linux."],
    ["Size", "About 25,000 lines of Rust, no crates; about 15,000 lines of the same compiler in Claw; a standard library under 900 lines; about 2,500 tests."],
  ];
  $("#index").innerHTML = I.map(([t, d]) => `<div><dt>${t}</dt><dd>${d}</dd></div>`).join("");
}

/* ═════════════ Paw ⇄ Claw: the whole manual in night print ═════════════ */
function setMode(m, ev) {
  const root = document.documentElement, btn = $("#mode");
  const apply = () => {
    root.dataset.mode = m;
    btn.setAttribute("aria-checked", m === "claw"); btn.dataset.label = m === "claw" ? "Claw" : "Paw";
    $('meta[name="theme-color"]').setAttribute("content", m === "claw" ? "#0e0e0d" : "#eeece6");
    try { localStorage.setItem("claw-mode", m); } catch {}
  };
  if (!document.startViewTransition || reduced) { apply(); return; }
  const r = (ev?.currentTarget || btn).getBoundingClientRect();
  const x = r.left + r.width / 2, y = r.top + r.height / 2;
  const R = Math.hypot(Math.max(x, innerWidth - x), Math.max(y, innerHeight - y));
  const t = document.startViewTransition(apply);
  t.ready.then(() => document.documentElement.animate({ clipPath: [`circle(0px at ${x}px ${y}px)`, `circle(${R}px at ${x}px ${y}px)`] },
    { duration: 700, easing: "cubic-bezier(.16, 1, .3, 1)", pseudoElement: "::view-transition-new(root)" }));
}
function mode() {
  const btn = $("#mode"), m = document.documentElement.dataset.mode || "paw";
  btn.setAttribute("aria-checked", m === "claw"); btn.dataset.label = m === "claw" ? "Claw" : "Paw";
  btn.addEventListener("click", e => setMode(document.documentElement.dataset.mode === "claw" ? "paw" : "claw", e));
}

/* ═════════════ running head, progress, nav ═════════════ */
function runningHead() {
  const secs = $$("[data-sec]"), num = $("#rh-sec"), title = $("#rh-title"), bar = $("#progress"), links = $$(".head-nav a");
  let cur = null;
  const upd = () => {
    let on = secs[0];
    for (const s of secs) if (s.getBoundingClientRect().top < innerHeight * .35) on = s;
    if (on !== cur) {
      cur = on; num.textContent = "§ " + on.dataset.sec; title.textContent = on.dataset.title;
      title.classList.remove("swap"); void title.offsetWidth; title.classList.add("swap");
      links.forEach(a => a.classList.toggle("on", a.getAttribute("href") === "#" + on.id));
    }
    bar.style.setProperty("--p", scrollY / Math.max(1, document.documentElement.scrollHeight - innerHeight));
  };
  addEventListener("scroll", upd, { passive: true }); upd();
}

/* ═════════════ copy, toast, palette, grid key ═════════════ */
function toast(m) { const t = $("#toast"); t.textContent = m; t.classList.add("show"); clearTimeout(toast.t); toast.t = setTimeout(() => t.classList.remove("show"), 2200); }
async function copy(text) { try { await navigator.clipboard.writeText(text); toast("Copied"); } catch { toast("Copy failed: select the text instead"); } }
function copies() {
  $$("[data-copy]").forEach(b => b.addEventListener("click", () => { copy(b.dataset.copy); b.classList.add("copied"); setTimeout(() => b.classList.remove("copied"), 1500); }));
}
function palette() {
  const pal = $("#palette"), inp = $("#palette-in"), list = $("#palette-list");
  let opener = null;
  const go = id => () => $(id).scrollIntoView({ behavior: reduced ? "auto" : "smooth" });
  const ITEMS = [
    ["The question", "§01", go("#question")], ["The language on one page", "§02", go("#prompt")], ["Paw and Claw", "§03", go("#gears")],
    ["The compiler marks your work", "§04", go("#desk")], ["Signatures and contracts", "§05", go("#truth")], ["Measured", "§06", go("#numbers")],
    ["Source to signed binary", "§07", go("#metal")], ["Everything in 0.10", "§08", go("#spec")], ["The fine print", "§09", go("#limits")], ["Start", "§10", go("#start")],
    ["claw test pick.claw", "run", () => window.__desk("claw test pick.claw")], ["claw why avg.claw", "run", () => window.__desk("claw why avg.claw")],
    ["claw fix move.claw", "run", () => window.__desk("claw fix move.claw")], ["claw audit paw_and_claw.claw", "run", () => window.__desk("claw audit paw_and_claw.claw")],
    ["Switch Paw / Claw mode", "action", () => $("#mode").click()], ["Show the layout grid", "action", () => document.documentElement.classList.toggle("grid-on")],
    ["Copy the install commands", "action", () => copy("cargo build --release --manifest-path compiler/Cargo.toml\npython3 tools/install.py\nclaw run examples/tour.claw")],
  ];
  let shown = [], sel = 0;
  const render = () => {
    const q = inp.value.toLowerCase().trim();
    shown = ITEMS.filter(([t, g]) => !q || (t + " " + g).toLowerCase().includes(q));
    sel = Math.min(sel, Math.max(0, shown.length - 1));
    list.innerHTML = shown.length ? shown.map(([t, g], i) => `<li role="option" aria-selected="${i === sel}" class="${i === sel ? "on" : ""}" data-i="${i}"><span>${esc(t)}</span><span class="g">${g}</span></li>`).join("")
      : `<li class="g">Nothing matches.</li>`;
    $$("li[data-i]", list).forEach(li => { li.addEventListener("click", () => run(+li.dataset.i)); li.addEventListener("pointermove", () => { if (sel !== +li.dataset.i) { sel = +li.dataset.i; render(); } }); });
    list.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  };
  const openP = () => { opener = document.activeElement; pal.hidden = false; inp.value = ""; sel = 0; render(); inp.focus(); };
  const close = () => { pal.hidden = true; opener?.focus?.(); };
  const run = i => { const it = shown[i]; close(); it?.[2](); };
  $("#open-palette").addEventListener("click", openP);
  pal.addEventListener("click", e => { if (e.target === pal) close(); });
  inp.addEventListener("input", () => { sel = 0; render(); });
  addEventListener("keydown", e => {
    const typing = /input|textarea/i.test(document.activeElement?.tagName || "");
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") { e.preventDefault(); pal.hidden ? openP() : close(); return; }
    if (pal.hidden) {
      if (typing) return;
      if (e.key === "/") { e.preventDefault(); openP(); }
      if (e.key.toLowerCase() === "g" && !e.metaKey && !e.ctrlKey) document.documentElement.classList.toggle("grid-on");
      return;
    }
    if (e.key === "Escape") close();
    else if (e.key === "ArrowDown") { sel = (sel + 1) % Math.max(1, shown.length); render(); e.preventDefault(); }
    else if (e.key === "ArrowUp") { sel = (sel - 1 + shown.length) % Math.max(1, shown.length); render(); e.preventDefault(); }
    else if (e.key === "Enter") { run(sel); e.preventDefault(); }
    else if (e.key === "Tab") e.preventDefault();
  });
}
function debounce(f, ms) { let t; return (...a) => { clearTimeout(t); t = setTimeout(() => f(...a), ms); }; }

/* ═════════════ boot ═════════════ */
function boot() {
  index();
  onePage();
  compare();
  desk();
  numbers();
  metal();
  mode();
  runningHead();
  reveals();
  copies();
  palette();
  kinetic();
  heroProof();
  requestAnimationFrame(() => requestAnimationFrame(() => document.body.classList.add("ready")));
}
const start = () => boot();
if (document.fonts && document.fonts.ready) document.fonts.ready.then(start, start); else start();
})();
