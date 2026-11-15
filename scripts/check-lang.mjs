#!/usr/bin/env node
// Code-language guard: pre-commit gate plus full-repository audit.
//
// The project language comes from .pengj-templates.json:
//   options.chinese_programming === false -> english mode:
//       identifiers must stay in Latin script, string literals must be English.
//   options.chinese_programming === true  -> chinese mode:
//       string literals must be Chinese (identifiers may stay English, which
//       the Chinese-programming convention explicitly allows as anchors).
//
// Tolerance is ratio-based and judged per file, so one drifting file cannot
// hide inside a large repository:
//   identifier violations / identifiers <= maxIdentRatio (default 0)
//   string violations     / strings     <= maxRatio      (default 0.01)
// A genuine, reviewed exception is certified in the source itself:
//   `lang-ok: <reason>`    exempts the line the comment sits on
//   `lang-allow: <reason>` exempts the whole file (i18n tables, CJK fixtures)
//
// Usage:
//   node scripts/check-lang.mjs [files...]   # staged files, or a full audit
//   node scripts/check-lang.mjs --self-test
//
// Flags:
//   --mode=english|chinese   override the manifest (env: PENGJ_CODE_LANG)
//   --max-ratio=0.01         string tolerance (env: PENGJ_LANG_MAX_RATIO)
//   --max-ident-ratio=0      identifier tolerance
//   --json                   machine-readable report
//   --list                   print every violation, not just the first ones

import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';

const MANIFEST = '.pengj-templates.json';
const SKIP_DIRS = new Set(['target', '.git', 'node_modules', 'third-party', 'generated']);
const DEFAULT_ROOTS = ['crates', 'apps', 'src', 'tests'];
const MAX_PRINTED = 80;

// "Not English" means a non-Latin script: CJK, kana, hangul, Cyrillic, Hebrew,
// Arabic, Devanagari, plus CJK/full-width punctuation. Latin accents and
// typographic marks (em dash, curly quotes) are still English copy, so they
// stay legal; Greek is excluded on purpose because math symbols are common.
const FOREIGN_SCRIPT =
  /[\u0400-\u04ff\u0590-\u05ff\u0600-\u06ff\u0900-\u097f\u2e80-\u2fff\u3000-\u303f\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7af\uf900-\ufaff\ufe30-\ufe4f\uff00-\uffef]/;
// Han ideographs: the compliance target in Chinese-programming mode.
const HAN = /[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/;
// CJK ideographs plus kana and compatibility blocks: what counts as CJK copy
// during an English audit (kana is included so Japanese copy is caught too).
const CJK = /[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/;
const ASCII_LETTER = /[A-Za-z]/;
// Any Unicode letter: used by Chinese mode to tell real copy (Latin, kana,
// Cyrillic...) apart from punctuation-only format strings.
const LETTER = /\p{L}/u;

const IDENT_START = /[A-Za-z_\u0080-\uffff]/;
const IDENT_CHAR = /[A-Za-z0-9_\u0080-\uffff]/;
const RAW_PREFIX = /^(?:b|c)?r(#*)"/;

const RUST_KEYWORDS = new Set([
  'abstract', 'as', 'async', 'await', 'become', 'box', 'break', 'const', 'continue',
  'crate', 'do', 'dyn', 'else', 'enum', 'extern', 'false', 'final', 'fn', 'for',
  'if', 'impl', 'in', 'let', 'loop', 'macro', 'macro_rules', 'match', 'mod', 'move',
  'mut', 'override', 'priv', 'pub', 'ref', 'return', 'self', 'Self', 'static',
  'struct', 'super', 'trait', 'true', 'try', 'type', 'typeof', 'union', 'unsafe',
  'unsized', 'use', 'virtual', 'where', 'while', 'yield',
]);

// Primitive type names are identifiers in the token stream but carry no
// language, so they would only dilute the denominator.
const PRIMITIVES = new Set([
  'bool', 'char', 'str', 'usize', 'isize',
  'i8', 'i16', 'i32', 'i64', 'i128', 'u8', 'u16', 'u32', 'u64', 'u128',
  'f32', 'f64',
]);

const MARKER_FILE = 'lang-allow:';
const MARKER_LINE = 'lang-ok:';

function walk(dir, out) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) walk(path.join(dir, entry.name), out);
    } else if (entry.name.endsWith('.rs')) {
      out.push(path.join(dir, entry.name));
    }
  }
  return out;
}

function collectTargets(files) {
  if (files.length > 0) {
    return files.filter((file) => file.endsWith('.rs')).map((file) => file.replaceAll('\\', '/'));
  }
  const roots = DEFAULT_ROOTS.map((root) => walk(root, [])).flat();
  return roots.map((file) => file.replaceAll('\\', '/'));
}

/**
 * Strip comments while collecting identifiers and string literals.
 *
 * Returns raw item lists; certification markers are resolved by the caller so
 * that a trailing `lang-ok:` comment can exempt earlier items on its line.
 */
function scan(source) {
  const identifiers = [];
  const strings = [];
  const lineOk = new Map();
  let fileAllow = null;
  let line = 1;
  let i = 0;
  const n = source.length;

  const countLines = (from, to) => {
    for (let k = from; k < to; k += 1) if (source[k] === '\n') line += 1;
  };

  const noteLineMarker = (text, atLine) => {
    const index = text.indexOf(MARKER_LINE);
    if (index === -1) return;
    const reason = text.slice(index + MARKER_LINE.length).trim();
    if (!lineOk.has(atLine)) lineOk.set(atLine, reason);
  };

  const noteFileMarker = (text) => {
    const index = text.indexOf(MARKER_FILE);
    if (index === -1) return;
    const reason = text.slice(index + MARKER_FILE.length).trim();
    if (fileAllow === null) fileAllow = reason;
  };

  while (i < n) {
    const c = source[i];
    const next = source[i + 1];

    if (c === '/' && next === '/') {
      const end = source.indexOf('\n', i);
      const text = source.slice(i, end === -1 ? n : end);
      noteFileMarker(text);
      noteLineMarker(text, line);
      i = end === -1 ? n : end;
      continue;
    }

    if (c === '/' && next === '*') {
      let depth = 0;
      let j = i;
      while (j < n) {
        if (source[j] === '/' && source[j + 1] === '*') {
          depth += 1;
          j += 2;
          continue;
        }
        if (source[j] === '*' && source[j + 1] === '/') {
          depth -= 1;
          j += 2;
          if (depth === 0) break;
          continue;
        }
        j += 1;
      }
      const text = source.slice(i, j);
      noteFileMarker(text);
      noteLineMarker(text, line);
      countLines(i, j);
      i = j;
      continue;
    }

    const raw = RAW_PREFIX.exec(source.slice(i, i + 8));
    if (raw && !IDENT_CHAR.test(source[i - 1] ?? '')) {
      const startLine = line;
      const bodyStart = i + raw[0].length;
      const closer = `"${raw[1]}`;
      const end = source.indexOf(closer, bodyStart);
      const body = source.slice(bodyStart, end === -1 ? n : end);
      strings.push({ line: startLine, text: body });
      const stop = end === -1 ? n : end + closer.length;
      countLines(i, stop);
      i = stop;
      continue;
    }

    if (c === '"') {
      const startLine = line;
      let j = i + 1;
      let body = '';
      while (j < n) {
        if (source[j] === '\\') {
          body += source[j] + (source[j + 1] ?? '');
          j += 2;
          continue;
        }
        if (source[j] === '"') break;
        body += source[j];
        j += 1;
      }
      strings.push({ line: startLine, text: body });
      const stop = Math.min(n, j + 1);
      countLines(i, stop);
      i = stop;
      continue;
    }

    if (c === "'") {
      const literal = /^'(?:\\.|[^\\'])'/.exec(source.slice(i, i + 8));
      if (literal) {
        strings.push({ line, text: literal[0] });
        countLines(i, i + literal[0].length);
        i += literal[0].length;
        continue;
      }
      i += 1;
      continue;
    }

    if (IDENT_START.test(c)) {
      let j = i;
      while (j < n && IDENT_CHAR.test(source[j])) j += 1;
      identifiers.push({ line, text: source.slice(i, j) });
      i = j;
      continue;
    }

    if (c === '\n') line += 1;
    i += 1;
  }

  return { identifiers, strings, fileAllow, lineOk };
}

/** True when a string literal carries human-readable text worth judging. */
function isLanguageString(text) {
  return ASCII_LETTER.test(text) || CJK.test(text) || FOREIGN_SCRIPT.test(text);
}

/**
 * Classify one file and return per-file counts.
 *
 * Exempt lines are dropped from both numerator and denominator: a certified
 * fixture should not make the remaining code look cleaner than it is.
 */
function classify(source, mode) {
  const { identifiers, strings, fileAllow, lineOk } = scan(source);
  const result = {
    fileAllow,
    lineOk,
    identifiers: 0,
    stringItems: 0,
    identViolations: [],
    stringViolations: [],
  };
  if (fileAllow !== null) return result;

  const exempt = (atLine) => lineOk.has(atLine);

  for (const token of identifiers) {
    if (exempt(token.line)) continue;
    if (RUST_KEYWORDS.has(token.text) || PRIMITIVES.has(token.text)) continue;
    result.identifiers += 1;
    if (mode === 'english' && FOREIGN_SCRIPT.test(token.text)) {
      result.identViolations.push(token);
    }
  }

  for (const token of strings) {
    if (exempt(token.line)) continue;
    const text = token.text;
    if (!isLanguageString(text)) continue;
    result.stringItems += 1;
    if (mode === 'english') {
      if (FOREIGN_SCRIPT.test(text)) result.stringViolations.push(token);
    } else if (!HAN.test(text) && LETTER.test(text)) {
      result.stringViolations.push(token);
    }
  }

  return result;
}

function allowance(items, ratio) {
  if (items === 0) return 0;
  // Guard against binary float noise: 200 * 0.01 must be exactly 2.
  return Math.floor(items * ratio + 1e-9);
}

function parseArgs(argv) {
  const options = { files: [], mode: null, maxRatio: null, maxIdentRatio: null, json: false, list: false, selfTest: false };
  for (const arg of argv) {
    if (arg === '--self-test') options.selfTest = true;
    else if (arg === '--json') options.json = true;
    else if (arg === '--list') options.list = true;
    else if (arg.startsWith('--mode=')) options.mode = arg.slice('--mode='.length);
    else if (arg.startsWith('--max-ratio=')) options.maxRatio = arg.slice('--max-ratio='.length);
    else if (arg.startsWith('--max-ident-ratio=')) options.maxIdentRatio = arg.slice('--max-ident-ratio='.length);
    else if (arg.startsWith('--')) {
      throw new Error(`unknown flag: ${arg}`);
    } else options.files.push(arg);
  }
  return options;
}

function readManifest() {
  try {
    return JSON.parse(readFileSync(MANIFEST, 'utf8'));
  } catch {
    return null;
  }
}

function resolveMode(flag) {
  if (flag) {
    if (flag !== 'english' && flag !== 'chinese') {
      throw new Error(`invalid --mode=${flag} (expected english|chinese)`);
    }
    return { mode: flag, source: '--mode' };
  }
  const env = process.env.PENGJ_CODE_LANG;
  if (env) {
    if (env !== 'english' && env !== 'chinese') {
      throw new Error(`invalid PENGJ_CODE_LANG=${env} (expected english|chinese)`);
    }
    return { mode: env, source: 'PENGJ_CODE_LANG' };
  }
  const manifest = readManifest();
  const flagValue = manifest?.options?.chinese_programming;
  if (typeof flagValue === 'boolean') {
    return { mode: flagValue ? 'chinese' : 'english', source: `${MANIFEST} options.chinese_programming` };
  }
  return null;
}

function resolveRatio(flag, envName, manifestKey) {
  const raw = flag ?? process.env[envName] ?? readManifest()?.options?.[manifestKey];
  if (raw === undefined || raw === null) return null;
  const value = Number(raw);
  if (!Number.isFinite(value) || value < 0 || value > 1) {
    throw new Error(`invalid ${manifestKey}=${raw} (expected a number in [0, 1])`);
  }
  return value;
}

function formatPercent(value) {
  return `${(value * 100).toFixed(value === 0 ? 0 : 2)}%`;
}

function runSelfTest() {
  const cases = [];
  const check = (name, condition) => {
    cases.push({ name, ok: Boolean(condition) });
  };

  // Sample literals use \u escapes so this file stays pure ASCII: an English
  // project forbids Chinese in any generated text file, while the scanner
  // still receives real CJK characters at runtime.
  const englishSample = [
    'fn title() -> &\'static str { "Hello" }',
    'fn \u6807\u9898() -> &\'static str { "\u4f60\u597d" }',
    'let x = "\u4f60\u597d"; // lang-ok: CJK input sample',
    '// \u4e2d\u6587\u6ce8\u91ca\u4e0d\u7b97\u8fdd\u89c4',
  ].join('\n');
  const english = classify(englishSample, 'english');
  check('english: ASCII identifier is not a violation', english.identViolations.length === 1);
  check('english: non-ASCII identifier is caught', english.identViolations[0]?.text === '\u6807\u9898');
  check('english: certified line is excluded', english.stringViolations.length === 1);
  check('english: certified string leaves the denominator', english.stringItems === 2);

  const chineseSample = [
    'fn title() { let label = "\u4fdd\u5b58"; }',
    'fn other() { let label = "Save"; }',
    'fn tag() { let label = "UTF-8"; } // lang-ok: technical token',
  ].join('\n');
  const chinese = classify(chineseSample, 'chinese');
  check('chinese: English copy is a violation', chinese.stringViolations.length === 1);
  check('chinese: English identifiers are allowed', chinese.identViolations.length === 0);
  check('chinese: certified technical token is excluded', chinese.stringItems === 2);

  const japanese = classify('let s = "\u3059\u308b";', 'chinese');
  check('chinese: kana-only copy still needs Han characters', japanese.stringViolations.length === 1);

  const allowedSample = '// lang-allow: i18n table\nfn f() { let _ = "\u4f60\u597d"; }';
  const allowed = classify(allowedSample, 'english');
  check('file certification bypasses the scan', allowed.fileAllow !== null && allowed.stringViolations.length === 0);

  const rawSample = 'let s = r#"\u4f60\u597d"#; let b = br#"ok"#; let c = "a";';
  const raw = classify(rawSample, 'english');
  check('raw strings are scanned', raw.stringViolations.length === 1);

  const noiseSample = 'let a = "{}"; let b = "->"; let c = "123";';
  const noise = classify(noiseSample, 'english');
  check('non-language strings are ignored', noise.stringItems === 0);

  const typography = classify('let s = "{} \u2014 {} items"; let q = "caf\u00e9";', 'english');
  check('typographic punctuation and Latin accents are legal English', typography.stringViolations.length === 0);

  const cyrillic = classify('let s = "\u041f\u0440\u0438\u0432\u0435\u0442";', 'english');
  check('non-Latin scripts are caught', cyrillic.stringViolations.length === 1);

  check('allowance floors partial items', allowance(120, 0.01) === 1 && allowance(99, 0.01) === 0);

  const failed = cases.filter((item) => !item.ok);
  for (const item of cases) {
    console.log(`${item.ok ? 'ok  ' : 'FAIL'} ${item.name}`);
  }
  if (failed.length > 0) {
    console.error(`[lang-guard] self-test failed: ${failed.length}/${cases.length}`);
    process.exit(1);
  }
  console.log(`[lang-guard] self-test passed: ${cases.length} cases`);
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.selfTest) {
    runSelfTest();
    return;
  }

  const resolved = resolveMode(options.mode);
  if (!resolved) {
    console.error(
      '[lang-guard] cannot determine the code language: pass --mode=english|chinese, ' +
        `set PENGJ_CODE_LANG, or add options.chinese_programming to ${MANIFEST}.`
    );
    process.exit(2);
  }
  const maxRatio = options.maxRatio === null ? resolveRatio(null, 'PENGJ_LANG_MAX_RATIO', 'lang_ratio') ?? 0.01 : Number(options.maxRatio);
  const maxIdentRatio = options.maxIdentRatio === null ? resolveRatio(null, 'PENGJ_LANG_MAX_IDENT_RATIO', 'lang_ident_ratio') ?? 0 : Number(options.maxIdentRatio);
  for (const [name, value] of [['--max-ratio', maxRatio], ['--max-ident-ratio', maxIdentRatio]]) {
    if (!Number.isFinite(value) || value < 0 || value > 1) {
      throw new Error(`invalid ${name}=${value} (expected a number in [0, 1])`);
    }
  }

  const targets = collectTargets(options.files);
  const reports = [];
  const markerIssues = [];
  for (const file of targets) {
    let source;
    try {
      source = readFileSync(file, 'utf8');
    } catch {
      // lefthook can hand over files deleted in the same commit.
      continue;
    }
    const result = classify(source, resolved.mode);
    if (result.fileAllow !== null && result.fileAllow.length === 0) {
      markerIssues.push(`${file}: ${MARKER_FILE} has no reason`);
    }
    for (const [atLine, reason] of result.lineOk) {
      if (reason.length === 0) markerIssues.push(`${file}:${atLine}: ${MARKER_LINE} has no reason`);
    }
    const identBudget = allowance(result.identifiers, maxIdentRatio);
    const stringBudget = allowance(result.stringItems, maxRatio);
    if (result.identViolations.length > identBudget || result.stringViolations.length > stringBudget) {
      reports.push({ ...result, file, identBudget, stringBudget });
    }
  }

  const summary = {
    mode: resolved.mode,
    modeSource: resolved.source,
    maxRatio,
    maxIdentRatio,
    scanned: targets.length,
    failing: reports.length,
    files: reports.map((report) => ({
      file: report.file,
      identifiers: report.identifiers,
      identViolations: report.identViolations.length,
      strings: report.stringItems,
      stringViolations: report.stringViolations.length,
      certified: report.fileAllow !== null,
    })),
    markers: markerIssues,
  };

  if (options.json) {
    console.log(JSON.stringify(summary, null, 2));
  } else if (reports.length > 0) {
    console.error(
      `[lang-guard] ${reports.length} file(s) exceed the tolerance ` +
        `(mode=${resolved.mode}, strings<=${formatPercent(maxRatio)}, identifiers<=${formatPercent(maxIdentRatio)}):`
    );
    for (const report of reports) {
      console.error(
        `  ${report.file}: identifiers ${report.identViolations.length}/${report.identifiers} ` +
          `(budget ${report.identBudget}), strings ${report.stringViolations.length}/${report.stringItems} ` +
          `(budget ${report.stringBudget})`
      );
      const shown = options.list ? [...report.identViolations, ...report.stringViolations] : [
        ...report.identViolations.slice(0, 3),
        ...report.stringViolations.slice(0, 3),
      ];
      for (const hit of shown.slice(0, MAX_PRINTED)) {
        console.error(`    ${report.file}:${hit.line}: "${hit.text.slice(0, 80)}"`);
      }
      const hidden = report.identViolations.length + report.stringViolations.length - shown.length;
      if (hidden > 0) console.error(`    ... ${hidden} more (re-run with --list)`);
    }
    if (markerIssues.length > 0) {
      console.error('[lang-guard] invalid certification markers:');
      for (const line of markerIssues) console.error(`  ${line}`);
    }
    console.error(
      '\n[lang-guard] Rules: identifiers and copy must use the project language; ' +
        `keep genuine exceptions to <=${formatPercent(maxRatio)} per file, ` +
        `or certify them with \`${MARKER_LINE} <reason>\` / \`${MARKER_FILE} <reason>\`.`
    );
    process.exitCode = 1;
  } else if (markerIssues.length > 0) {
    console.error(`[lang-guard] ${markerIssues.length} invalid certification marker(s):`);
    for (const line of markerIssues) console.error(`  ${line}`);
    console.error(`[lang-guard] \`${MARKER_FILE}\` and \`${MARKER_LINE}\` require a reason after the colon.`);
    process.exitCode = 1;
  } else {
    console.log(`[lang-guard] passed: ${targets.length} Rust file(s), mode=${resolved.mode}.`);
  }
}

try {
  main();
} catch (error) {
  console.error(`[lang-guard] ${error.message}`);
  process.exit(2);
}