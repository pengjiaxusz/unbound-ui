#!/usr/bin/env node
// Project-local MCP bootstrap: verify / install / sync, using only Node built-ins plus the
// local uv and git.
//
// Design choice: no bundled executables.
//   - An exe needs one copy per platform (win/mac/linux x x64/arm64); committing them bloats
//     the repo and cannot be reviewed;
//   - antivirus software flags unknown binaries, and users cannot diff them;
//   - the actual install steps are few (uv tool install / copy a directory / write JSON), so a
//     script expresses them well enough — and when something breaks you can read the source.
//
// Single source of truth: the project-root .mcp.json (commands are bare names, portable across
// machines).
// Install recipes: the RECIPES table below, plus an optional project file
// scripts/mcp/recipes.local.json (same shape, overrides by name).
//
// Usage (run from the project root, or any subdirectory — the script walks up to find .mcp.json):
//   node .agents/skills/mcp-bootstrap/scripts/ensure-mcp.mjs              # = doctor
//   node .agents/skills/mcp-bootstrap/scripts/ensure-mcp.mjs doctor [--static]
//   node .agents/skills/mcp-bootstrap/scripts/ensure-mcp.mjs install [name] [--global]
//   node .agents/skills/mcp-bootstrap/scripts/ensure-mcp.mjs sync [--write]
//
// Exit codes: 0 everything is fine; 1 there are problems to fix (doctor).

import { spawn, spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';

const SOURCE_CACHE = join(homedir(), '.pengj-mcp', 'src');
const APPDATA = process.env.APPDATA ?? join(homedir(), 'AppData', 'Roaming');

// Recipes that need a real install plus a side artifact. Any other server is assumed to be on
// PATH already: doctor only verifies it, install does nothing.
const RECIPES = {
  'renderdoc-mcp': {
    type: 'uv-git',
    // Private fork: the upstream extension hard-depends on PySide2, while a self-built RenderDoc
    // is often configured with PYSIDE2_ENABLED=0, so the upstream copy fails at import time.
    // The fork adds a thread-polling fallback for builds without PySide2.
    source: 'git+https://github.com/pengjiaxusz/RenderDocMCP',
    entry: 'renderdoc-mcp',
    // The global config uses the same name as the pengj-templates mcp subcommand, so one server
    // never ends up written twice under two names.
    globalName: 'renderdoc',
    extension: {
      from: 'renderdoc_extension',
      to: join(APPDATA, 'qrenderdoc', 'extensions', 'renderdoc_mcp_bridge'),
    },
    enable: {
      file: join(APPDATA, 'qrenderdoc', 'UI.config'),
      key: 'AlwaysLoad_Extensions',
      value: 'renderdoc_mcp_bridge',
    },
    requires: ['qrenderdoc'],
  },
  'playwright-mcp': {
    type: 'none',
    hint: 'npm i -g @playwright/mcp (or run it through npx @playwright/mcp)',
  },
};

// Per-client project configs derived from .mcp.json; the variant picks the remote field name.
// agy / Trae mcp_config.json only accept serverUrl, never url.
const TARGETS = [
  { file: '.agents/mcp.json', variant: 'url' },
  { file: '.trae/mcp.json', variant: 'url' },
  { file: '.agents/mcp_config.json', variant: 'serverUrl' },
  { file: '.trae/mcp_config.json', variant: 'serverUrl' },
];

// Clients that only read a global config (a project-level config is ignored by them; --global
// writes one copy).
const GLOBAL_CLIENTS = [
  {
    id: 'workbuddy',
    path: join(homedir(), '.workbuddy', 'mcp.json'),
    shape: 'mcpServers',
    hint: 'click "Trust" once in the WorkBuddy MCP settings',
  },
  {
    id: 'opencode',
    path: join(homedir(), '.config', 'opencode', 'opencode.json'),
    shape: 'opencode',
    hint: 'open a new opencode session',
  },
  {
    id: 'claude-code',
    path: join(homedir(), '.claude.json'),
    shape: 'mcpServers',
    hint: 'read on the next Claude Code session',
  },
  {
    id: 'agy',
    path: join(homedir(), '.gemini', 'config', 'mcp_config.json'),
    shape: 'mcpServers',
    hint: 'run /mcp reload in agy',
  },
  {
    id: 'trae',
    path: join(APPDATA, 'Trae CN', 'User', 'mcp.json'),
    shape: 'mcpServers',
    hint: 'restart Trae CN',
  },
];

// `--root <dir>` takes a value, so it must be pulled out before positionals are scanned —
// otherwise the directory would be read as the command name.
const rawArgs = process.argv.slice(2);
const rootIndex = rawArgs.indexOf('--root');
const ROOT_FLAG = rootIndex >= 0 ? rawArgs[rootIndex + 1] : null;
const argv = rootIndex >= 0 ? rawArgs.filter((_, index) => index !== rootIndex && index !== rootIndex + 1) : rawArgs;

const command = argv.find((arg) => !arg.startsWith('-')) ?? 'doctor';
const flags = new Set(argv.filter((arg) => arg.startsWith('-')));
const nameFilter = argv.find((arg, index) => index > 0 && !arg.startsWith('-') && arg !== command);

/** Project root: --root wins, otherwise walk up from the current directory looking for .mcp.json */
function findRoot() {
  if (ROOT_FLAG) return resolve(ROOT_FLAG);
  let current = process.cwd();
  for (let level = 0; level < 6; level += 1) {
    if (existsSync(join(current, '.mcp.json'))) return current;
    const parent = dirname(current);
    if (parent === current) break;
    current = parent;
  }
  return process.cwd();
}

const ROOT = findRoot();
const MANIFEST_FILE = join(ROOT, '.mcp.json');

// ---------- primitives ----------

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

/** Optional project-local recipes: scripts/mcp/recipes.local.json, overrides by name */
let localRecipeCache;
function localRecipes() {
  if (localRecipeCache !== undefined) return localRecipeCache;
  const path = join(ROOT, 'scripts', 'mcp', 'recipes.local.json');
  if (!existsSync(path)) {
    localRecipeCache = {};
    return localRecipeCache;
  }
  try {
    localRecipeCache = readJson(path);
  } catch (error) {
    console.error(`[mcp] cannot parse local recipes, ignoring: ${error.message}`);
    localRecipeCache = {};
  }
  return localRecipeCache;
}

function recipeFor(name) {
  return localRecipes()[name] ?? RECIPES[name];
}

/** Recipe paths accept `~`, `%VAR%` and `$VAR`, matching how the rest of the project is configured */
function expandPath(value) {
  if (!value) return value;
  return value
    .replace(/^~/, homedir())
    .replace(/%([^%]+)%/g, (raw, name) => process.env[name] ?? raw)
    .replace(/\$\{?([A-Za-z_][A-Za-z0-9_]*)\}?/g, (raw, name) => process.env[name] ?? raw);
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
}

/** Byte-for-byte comparison with the intended output, so key order churn never rewrites a file */
function renderConfig(manifest, variant) {
  const result = JSON.parse(JSON.stringify(manifest));
  if (variant === 'serverUrl') {
    for (const item of Object.values(result.mcpServers ?? {})) {
      if (item.url !== undefined) {
        item.serverUrl = item.url;
        delete item.url;
      }
    }
  }
  return `${JSON.stringify(result, null, 2)}\n`;
}

/** A project without .mcp.json simply declares no project-level MCP — not an error */
function hasManifest() {
  return existsSync(MANIFEST_FILE);
}

function readManifest() {
  if (!hasManifest()) return { mcpServers: {} };
  return readJson(MANIFEST_FILE);
}

function noticeNoManifest() {
  console.log(
    '[mcp] no .mcp.json in this project, so there is no project-level MCP (to enable: create {"mcpServers": {}} and add entries)',
  );
}

/** Resolve an executable on PATH (including the uv shim dir); on Windows .exe beats an extensionless script */
function resolveCommand(name) {
  if (!name) return null;
  if (/[\\/]/.test(name)) return existsSync(name) ? name : null;

  const dirs = (process.env.PATH ?? '').split(delimiter).filter(Boolean);
  dirs.push(join(homedir(), '.local', 'bin'));
  const uvShim = uvBinDir();
  if (uvShim) dirs.push(uvShim);

  const extensions = process.platform === 'win32' ? ['.exe', '.cmd', '.bat', ''] : [''];
  for (const dir of dirs) {
    for (const extension of extensions) {
      const candidate = join(dir, `${name}${extension}`);
      if (existsSync(candidate)) return candidate;
    }
  }
  return null;
}

let uvBinDirCache;
function uvBinDir() {
  if (uvBinDirCache !== undefined) return uvBinDirCache;
  const result = spawnSync('uv', ['tool', 'dir', '--bin'], { encoding: 'utf8' });
  uvBinDirCache = result.status === 0 && result.stdout.trim() ? result.stdout.trim() : null;
  return uvBinDirCache;
}

function uvInstalled(entry) {
  const result = spawnSync('uv', ['tool', 'list'], { encoding: 'utf8' });
  if (result.status !== 0) return false;
  return result.stdout
    .split(/\r?\n/)
    .map((line) => line.replace(/^[-\s]+/, '').split(/\s+/)[0])
    .filter(Boolean)
    .includes(entry);
}

/** Normalize a git source from a recipe: drop the `git+` prefix and any `#ref` fragment */
function normalizeSource(source) {
  return source ? source.replace(/^git\+/, '').split('#')[0] : null;
}

/** Read uv's install receipt and return the git source it was installed from; null when unknown */
function installedSource(entry) {
  const dir = spawnSync('uv', ['tool', 'dir'], { encoding: 'utf8' }).stdout?.trim();
  if (!dir) return null;
  const receipt = join(dir, entry, 'uv-receipt.toml');
  if (!existsSync(receipt)) return null;
  const hit = readFileSync(receipt, 'utf8').match(/git\s*=\s*"([^"]+)"/);
  return hit ? normalizeSource(hit[1]) : null;
}

/** .cmd/.bat must be launched through cmd.exe; POSIX runs them directly */
function spawnArgs(commandPath, args) {
  if (process.platform === 'win32' && /\.(cmd|bat)$/i.test(commandPath)) {
    return {
      executable: process.env.ComSpec ?? 'cmd.exe',
      args: ['/d', '/s', '/c', commandPath, ...args],
      shell: false,
    };
  }
  return { executable: commandPath, args, shell: false };
}

// ---------- sync: .mcp.json -> the other four ----------

function syncPlan(manifest) {
  return TARGETS.flatMap((target) => {
    const absolute = join(ROOT, target.file);
    // Skip when the client directory does not exist at all, so we never invent a .trae/ folder
    if (!existsSync(absolute) && !existsSync(dirname(absolute))) return [];
    const expected = renderConfig(manifest, target.variant);
    const actual = existsSync(absolute) ? readFileSync(absolute, 'utf8') : null;
    return [{ ...target, expected, drifted: actual !== expected }];
  });
}

function runSync(manifest, write) {
  if (!hasManifest()) {
    noticeNoManifest();
    return 0;
  }
  let changes = 0;
  const plan = syncPlan(manifest);
  for (const item of plan) {
    if (!item.drifted) continue;
    changes += 1;
    if (write) {
      writeJson(join(ROOT, item.file), JSON.parse(item.expected));
      console.log(`[mcp] synced ${item.file} (variant ${item.variant})`);
    } else {
      console.log(`[mcp] pending ${item.file} (variant ${item.variant})`);
    }
  }
  if (plan.length === 0) console.log('[mcp] no derived configs to sync (client directories missing)');
  else if (changes === 0) console.log(`[mcp] all ${plan.length} derived configs match .mcp.json`);
  else if (!write) console.log('[mcp] add --write to apply');
  return changes;
}

// ---------- handshake: a real stdio JSON-RPC round trip ----------

// The timeout is generous at 20s: a freshly installed uv tool has to build its bytecode cache on
// the first cold start, and Windows antivirus scans the new exe — measured 8s+ the first time and
// under 1.5s the second, so an 8s cut-off reports false "handshake timeout" failures.
function handshake(commandPath, args, timeoutMs = 20000) {
  return new Promise((done) => {
    const startedAt = Date.now();
    const { executable, args: spawnArgv } = spawnArgs(commandPath, args);
    let child;
    try {
      child = spawn(executable, spawnArgv, { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
    } catch (error) {
      return done({ ok: false, error: error.message, elapsed: 0 });
    }

    let buffer = '';
    let stderrTail = [];
    let settled = false;
    let serverName = '?';
    let serverVersion = '';
    let timer;

    const finish = (result) => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      try {
        child.kill();
      } catch {
        /* already exited */
      }
      done({ ...result, elapsed: Date.now() - startedAt });
    };

    const send = (payload) => {
      try {
        child.stdin.write(`${JSON.stringify(payload)}\n`);
      } catch {
        /* the process is gone; the exit handler settles this */
      }
    };

    timer = setTimeout(
      () => finish({ ok: false, error: `handshake timeout (${timeoutMs}ms)` }),
      timeoutMs,
    );

    child.stdin.on('error', () => {
      /* EPIPE: the process exited early, the exit event handles it */
    });
    child.stderr.on('data', (chunk) => {
      stderrTail.push(...chunk.toString().split(/\r?\n/).filter(Boolean));
      if (stderrTail.length > 20) stderrTail = stderrTail.slice(-20);
    });
    child.on('error', (error) => finish({ ok: false, error: `cannot start process: ${error.message}` }));
    child.on('exit', () => {
      if (settled) return;
      const tail = stderrTail.length > 0 ? ` (stderr: ${stderrTail.slice(-3).join(' | ')})` : '';
      finish({ ok: false, error: `process exited before answering${tail}` });
    });

    send({
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2025-06-18',
        capabilities: {},
        clientInfo: { name: 'ensure-mcp', version: '1.0.0' },
      },
    });

    // Non-JSON lines (startup banners, logs) are skipped
    child.stdout.on('data', (chunk) => {
      buffer += chunk.toString();
      let newline;
      while ((newline = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, newline).trim();
        buffer = buffer.slice(newline + 1);
        if (!line) continue;
        let message;
        try {
          message = JSON.parse(line);
        } catch {
          continue;
        }
        if (message.id === 1) {
          if (message.error) {
            finish({ ok: false, error: `server returned an error: ${JSON.stringify(message.error)}` });
            return;
          }
          serverName = message.result?.serverInfo?.name ?? '?';
          serverVersion = message.result?.serverInfo?.version ?? '';
          send({ jsonrpc: '2.0', method: 'notifications/initialized' });
          send({ jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} });
          continue;
        }
        if (message.id === 2) {
          if (message.error) {
            finish({ ok: false, error: `tools/list failed: ${JSON.stringify(message.error)}` });
            return;
          }
          const toolCount = Array.isArray(message.result?.tools) ? message.result.tools.length : 0;
          finish({ ok: true, name: serverName, version: serverVersion, toolCount });
          return;
        }
      }
    });
  });
}

// ---------- doctor ----------

function inspectExtension(name) {
  const recipe = recipeFor(name);
  if (!recipe?.extension) return null;
  const missing = !existsSync(expandPath(recipe.extension.to));
  const configFile = recipe.enable ? expandPath(recipe.enable.file) : null;
  const configured =
    configFile && existsSync(configFile)
      ? (readJson(configFile)[recipe.enable.key] ?? []).includes(recipe.enable.value)
      : null;
  return { missing, configured };
}

async function doctor() {
  if (!hasManifest()) {
    noticeNoManifest();
    return 0;
  }
  const manifest = readManifest();
  const servers = Object.entries(manifest.mcpServers ?? {});
  const problems = [];

  for (const item of syncPlan(manifest).filter((entry) => entry.drifted)) {
    problems.push(`${item.file} does not match .mcp.json`);
  }

  console.log(`[mcp] ${servers.length} server(s) declared in .mcp.json`);
  const handshaken = new Map();
  for (const [name, definition] of servers) {
    if (definition.url) {
      console.log(`  [remote] ${name} -> ${definition.url}`);
      continue;
    }
    const commandPath = resolveCommand(definition.command);
    if (!commandPath) {
      console.log(`  [missing] ${name} -> ${definition.command} (not found on PATH)`);
      problems.push(`${name}: command unavailable: ${definition.command}`);
      continue;
    }
    if (flags.has('--static') || handshaken.has(commandPath)) {
      console.log(`  [ok] ${name} -> ${commandPath}`);
      continue;
    }
    const result = await handshake(commandPath, definition.args ?? []);
    handshaken.set(commandPath, result);
    if (result.ok) {
      console.log(
        `  [ok] ${name} -> ${result.name} ${result.version} · ${result.toolCount} tools · ${result.elapsed}ms`,
      );
    } else {
      console.log(`  [fail] ${name} -> ${result.error}`);
      problems.push(`${name}: handshake failed: ${result.error}`);
    }
  }

  // Side artifacts: without the extension the server answers but has no data
  for (const [name] of servers) {
    const check = inspectExtension(name);
    if (!check) continue;
    if (check.missing) {
      console.log(`  [pending] ${name}: extension not in place`);
      problems.push(`${name}: extension not installed, run install`);
    } else if (check.configured === false) {
      console.log(`  [pending] ${name}: extension copied, but the host has not enabled it (AlwaysLoad_Extensions)`);
      problems.push(`${name}: extension not loaded by the host, run install or enable it manually`);
    } else {
      console.log(`  [ok] ${name}: extension in place${check.configured ? ', host enabled it' : ''}`);
    }
  }

  if (problems.length === 0) {
    console.log('[mcp] everything is ready');
    return 0;
  }
  console.log(`[mcp] ${problems.length} problem(s):`);
  for (const item of problems) console.log(`  - ${item}`);
  return 1;
}

// ---------- install ----------

function ensureSource(name, recipe) {
  const cache = join(SOURCE_CACHE, name.replace(/-mcp$/, ''));
  const from = recipe.extension?.from;

  const source = recipe.source?.replace(/^git\+/, '').split('#')[0];
  if (!source) {
    console.log(`[mcp] ${name}: no source and no git repo, skipping the extension`);
    return null;
  }

  if (from && existsSync(join(cache, from))) {
    // A changed source (upstream -> own fork) must not reuse the old clone, or the installed
    // extension stays stale
    const current = spawnSync('git', ['-C', cache, 'remote', 'get-url', 'origin'], { encoding: 'utf8' });
    const currentSource = (current.stdout ?? '').trim();
    if (currentSource === source) return cache;
    console.log(`[mcp] ${name}: source cache points at ${currentSource || 'unknown'}, not the recipe, re-cloning`);
    rmSync(cache, { recursive: true, force: true });
  }

  console.log(`[mcp] cloning source -> ${cache}`);
  mkdirSync(dirname(cache), { recursive: true });
  const result = spawnSync('git', ['clone', '--depth', '1', source, cache], { stdio: 'inherit' });
  if (result.status !== 0) {
    console.error(`[mcp] ${name}: source clone failed`);
    return null;
  }
  return cache;
}

function writeGlobal(name, commandPath) {
  for (const client of GLOBAL_CLIENTS) {
    const before = existsSync(client.path) ? readFileSync(client.path, 'utf8') : null;
    const document = before ? JSON.parse(before) : {};
    if (client.shape === 'opencode') {
      document.mcp ??= {};
      document.mcp.servers ??= {};
      document.mcp.servers[name] = { type: 'local', command: [commandPath], enabled: true };
    } else {
      document.mcpServers ??= {};
      document.mcpServers[name] = { command: commandPath, args: [] };
    }
    const after = `${JSON.stringify(document, null, 2)}\n`;
    if (after === before) {
      console.log(`  [global] ${client.id} already up to date (${client.path})`);
      continue;
    }
    if (before !== null) {
      const backup = `${client.path}.pengj-mcp.bak`;
      if (!existsSync(backup)) cpSync(client.path, backup);
    }
    writeJson(client.path, document);
    console.log(`  [global] ${client.id} -> ${client.path} (${client.hint})`);
  }
}

function install(targetName) {
  if (!hasManifest()) {
    noticeNoManifest();
    return 0;
  }
  const manifest = readManifest();
  const servers = Object.entries(manifest.mcpServers ?? {}).filter(
    ([name]) => !targetName || name === targetName,
  );
  if (servers.length === 0) {
    console.error(`[mcp] no matching server: ${targetName}`);
    return 1;
  }

  let failures = 0;
  for (const [name, definition] of servers) {
    const recipe = recipeFor(name);
    if (!recipe) {
      console.log(`[mcp] ${name}: no install recipe, skipping (install it manually if needed)`);
      continue;
    }
    if (recipe.type === 'none') {
      console.log(`[mcp] ${name}: ${recipe.hint ?? 'no install needed'}`);
      continue;
    }

    // 1) the server itself
    if (recipe.type === 'uv-git') {
      // A changed source (upstream -> own fork) must be reinstalled, or the tool keeps old code
      const expectedSource = normalizeSource(recipe.source);
      const currentSource = installedSource(recipe.entry);
      const sourceMismatch = Boolean(expectedSource && currentSource && currentSource !== expectedSource);
      if (uvInstalled(recipe.entry) && !sourceMismatch) {
        console.log(`[mcp] ${name}: server already installed (uv tool)`);
      } else {
        if (sourceMismatch) {
          console.log(`[mcp] ${name}: installed from ${currentSource}, not the recipe, reinstalling...`);
        } else {
          console.log(`[mcp] ${name}: installing the server (uv tool install)...`);
        }
        const result = spawnSync(
          'uv',
          ['tool', 'install', '--force', '--from', recipe.source, recipe.entry],
          { stdio: 'inherit' },
        );
        if (result.status !== 0) {
          console.error(`[mcp] ${name}: server install failed`);
          failures += 1;
          continue;
        }
      }
    }

    // 2) side artifact
    if (recipe.extension) {
      const cache = ensureSource(name, recipe);
      const fromDir = cache && join(cache, recipe.extension.from);
      const toDir = expandPath(recipe.extension.to);
      if (fromDir && existsSync(fromDir)) {
        if (existsSync(toDir)) rmSync(toDir, { recursive: true, force: true });
        mkdirSync(dirname(toDir), { recursive: true });
        cpSync(fromDir, toDir, {
          recursive: true,
          filter: (path) => !path.includes('__pycache__') && !path.endsWith('.pyc'),
        });
        console.log(`[mcp] ${name}: extension -> ${toDir}`);
      }
    }

    // 3) let the host load it
    const enableFile = recipe.enable ? expandPath(recipe.enable.file) : null;
    if (recipe.enable && existsSync(enableFile)) {
      if (hostRunning(recipe.requires)) {
        console.log(`[mcp] ${name}: ${recipe.requires.join(',')} is running and would overwrite the config on exit, skipping enable`);
      } else {
        const config = readJson(enableFile);
        const entries = Array.isArray(config[recipe.enable.key]) ? config[recipe.enable.key] : [];
        if (!entries.includes(recipe.enable.value)) {
          entries.push(recipe.enable.value);
          config[recipe.enable.key] = entries;
          writeJson(enableFile, config);
          console.log(`[mcp] ${name}: wrote ${recipe.enable.key} += ${recipe.enable.value}`);
        } else {
          console.log(`[mcp] ${name}: host already loads the extension`);
        }
      }
    }

    // 4) optional global config
    if (flags.has('--global')) {
      const commandPath = resolveCommand(definition.command);
      if (!commandPath) {
        console.error(`[mcp] ${name}: command still unavailable, cannot write the global config`);
        failures += 1;
      } else {
        writeGlobal(recipe.globalName ?? name, commandPath);
      }
    }
  }
  return failures === 0 ? 0 : 1;
}

function hostRunning(required = []) {
  if (process.platform !== 'win32') return false;
  return required.some((name) => {
    const result = spawnSync('tasklist', ['/FI', `IMAGENAME eq ${name}.exe`, '/NH'], {
      encoding: 'utf8',
    });
    return (result.stdout ?? '').toLowerCase().includes(`${name}.exe`.toLowerCase());
  });
}

// ---------- entry ----------

const COMMANDS = {
  doctor,
  install,
  // A successful sync is a success; the change count is not an exit code (a shell would read it
  // as a failure)
  sync: () => {
    runSync(readManifest(), flags.has('--write'));
    return 0;
  },
};

if (!COMMANDS[command]) {
  console.error(`[mcp] unknown command: ${command} (available: doctor / install / sync)`);
  process.exit(2);
}

Promise.resolve(COMMANDS[command](nameFilter))
  .then((code) => process.exit(Number(code) || 0))
  .catch((error) => {
    console.error(`[mcp] failed: ${error.message}`);
    process.exit(1);
  });