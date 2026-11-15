#!/usr/bin/env node
// unbound-ui 中文字符守卫（pre-commit）。
//
// 项目约定（见 .pengj-templates.json 的 chinese_programming: false）：
//   - 标识符一律用英文，出现中文直接拒绝，不存在例外。
//   - 字符串 / 字符字面量里出现中文（含中文标点）必须“持证上岗”：
//       * 行尾注释写 `zh-ok: <理由>` —— 只放行该行；
//       * 文件任意注释里写 `zh-strings-allow: <理由>` —— 放行整个文件（i18n 翻译表等）。
// 注释本身必须是中文，因此注释内容不参与扫描。
//
// 用法：node scripts/check-zh-strings.mjs [文件...]
//   不给文件时扫描 crates/ 与 apps/ 下的全部 .rs（用于全量审计）。

import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';

// 中文汉字、中文标点、兼容表意文字、全角符号，以及日文假名（同样属于非 ASCII 文案）。
const CJK = /[\u2e80-\u2fff\u3000-\u303f\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\ufe30-\ufe4f\uff00-\uffef]/;
const IDENT_START = /[A-Za-z_\u0080-\uffff]/;
const IDENT_CHAR = /[A-Za-z0-9_\u0080-\uffff]/;
const RAW_PREFIX = /^(?:b|c)?r(#*)"/;

const SKIP_DIRS = new Set(['target', '.git', 'node_modules', 'third-party']);

function walk(dir, out) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) walk(path.join(dir, entry.name), out);
    } else if (entry.name.endsWith('.rs')) {
      out.push(path.join(dir, entry.name));
    }
  }
  return out;
}

/**
 * 逐字符扫描 Rust 源码，把注释剥掉后分别收集“中文标识符”与“含中文的字符串字面量”。
 * 同时记录认证标记：文件级 `zh-strings-allow` 与行级 `zh-ok`。
 */
function scan(source) {
  const identifiers = [];
  const strings = [];
  const lineOk = new Set();
  let fileAllow = false;
  let line = 1;
  let i = 0;
  const n = source.length;

  const countLines = (from, to) => {
    for (let k = from; k < to; k += 1) if (source[k] === '\n') line += 1;
  };

  while (i < n) {
    const c = source[i];
    const next = source[i + 1];

    if (c === '/' && next === '/') {
      const end = source.indexOf('\n', i);
      const text = source.slice(i, end === -1 ? n : end);
      if (text.includes('zh-strings-allow')) fileAllow = true;
      if (text.includes('zh-ok')) lineOk.add(line);
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
        // 块注释里的认证标记按起始行登记，够用且不会漏判。
        j += 1;
      }
      const text = source.slice(i, j);
      if (text.includes('zh-strings-allow')) fileAllow = true;
      if (text.includes('zh-ok')) lineOk.add(line);
      countLines(i, j);
      i = j;
      continue;
    }

    const raw = RAW_PREFIX.exec(source.slice(i, i + 8));
    if (raw && !IDENT_CHAR.test(source[i - 1] ?? '')) {
      const hashes = raw[1];
      const startLine = line;
      const bodyStart = i + raw[0].length;
      const closer = `"${hashes}`;
      const end = source.indexOf(closer, bodyStart);
      const body = source.slice(bodyStart, end === -1 ? n : end);
      if (CJK.test(body)) strings.push({ line: startLine, text: body.slice(0, 80) });
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
      if (CJK.test(body)) strings.push({ line: startLine, text: body.slice(0, 80) });
      const stop = Math.min(n, j + 1);
      countLines(i, stop);
      i = stop;
      continue;
    }

    if (c === "'") {
      const literal = /^'(?:\\.|[^\\'])'/.exec(source.slice(i, i + 8));
      if (literal) {
        if (CJK.test(literal[0])) strings.push({ line, text: literal[0] });
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
      const token = source.slice(i, j);
      if (CJK.test(token)) identifiers.push({ line, text: token });
      i = j;
      continue;
    }

    if (c === '\n') line += 1;
    i += 1;
  }

  return { identifiers, strings, fileAllow, lineOk };
}

const args = process.argv.slice(2);
const roots = ['crates', 'apps'];
const targets = args.length
  ? args.filter((file) => file.endsWith('.rs')).map((file) => file.replaceAll('\\', '/'))
  : roots.flatMap((root) => walk(root, [])).map((file) => file.replaceAll('\\', '/'));

const failures = [];
for (const file of targets) {
  let source = '';
  try {
    source = readFileSync(file, 'utf8');
  } catch {
    // lefthook 可能传入已删除或不可读的路径，跳过即可。
    continue;
  }
  const { identifiers, strings, fileAllow, lineOk } = scan(source);
  for (const hit of identifiers) {
    failures.push(`${file}:${hit.line}: 标识符 "${hit.text}" 含中文，必须改成英文`);
  }
  if (fileAllow) continue;
  for (const hit of strings) {
    if (lineOk.has(hit.line)) continue;
    failures.push(
      `${file}:${hit.line}: 字符串 "${hit.text}" 含中文，需在本行注释加 \`zh-ok: 理由\`，` +
        `或在文件头加 \`zh-strings-allow: 理由\``
    );
  }
}

if (failures.length > 0) {
  console.error(`[zh-strings] 发现 ${failures.length} 处中文违规：`);
  for (const line of failures) console.error(`  ${line}`);
  console.error(
    '\n[zh-strings] 规则：标识符必须英文；中文串仅限 i18n 翻译表等经认证的场景。'
  );
  process.exit(1);
}

console.log(`[zh-strings] 通过：检查 ${targets.length} 个 Rust 文件，未发现未认证的中文。`);
