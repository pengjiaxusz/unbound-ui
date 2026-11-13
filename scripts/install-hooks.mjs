import { execFileSync, execSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';

// prepare 阶段安装 lefthook 钩子。安装失败必须让 install 失败，
// 绝不吞异常：否则 commit-msg 钩子缺失时提交门禁会静默失效。

function git(args, options = {}) {
  return execFileSync('git', args, { encoding: 'utf8', ...options });
}

let inWorkTree = true;
try {
  execFileSync('git', ['rev-parse', '--git-dir'], { stdio: 'ignore' });
} catch {
  inWorkTree = false;
}

if (!inWorkTree) {
  // 打包产物 / 无 .git 的 CI 环境：跳过安装是正常情况，不算失败
  console.log('[lefthook] 当前不是 git 仓库，跳过钩子安装');
  process.exit(0);
}

let hooksPath = '';
try {
  hooksPath = git(['config', '--get', 'core.hooksPath']).trim();
} catch {
  // 未配置 core.hooksPath，使用 git 默认 hooks 目录
}

// lefthook 默认拒绝自定义 core.hooksPath；只有 --force 才会装进该目录
const installArgs = hooksPath ? ['install', '--force'] : ['install'];

try {
  // 走 shell 才能解析 pnpm 在 node_modules/.bin 下生成的 lefthook.cmd（Windows）
  execSync(`lefthook ${installArgs.join(' ')}`, { stdio: 'inherit' });
} catch (error) {
  const forceSuffix = hooksPath ? ' --force' : '';
  console.error(
    `[lefthook] 钩子安装失败（core.hooksPath=${hooksPath || '默认'}）。\n` +
      `  请手动执行：pnpm exec lefthook install${forceSuffix}\n` +
      '  常见原因：hooksPath 指向不存在的目录、目录不可写或 git 版本过旧。'
  );
  process.exit(error.status || 1);
}

// 装完必须确认 commit-msg 真的落在 git 会读取的目录里
const repoRoot = git(['rev-parse', '--show-toplevel']).trim();
const defaultHooks = git(['rev-parse', '--git-path', 'hooks']).trim();
const hooksDir = hooksPath
  ? path.resolve(repoRoot, hooksPath)
  : path.resolve(repoRoot, defaultHooks);
const commitMsgHook = path.join(hooksDir, 'commit-msg');

if (!existsSync(commitMsgHook)) {
  console.error(
    `[lefthook] 安装后未在 ${hooksDir} 找到 commit-msg 钩子；提交门禁不会生效。`
  );
  process.exit(1);
}

console.log(`[lefthook] 提交门禁已就绪：${commitMsgHook}`);