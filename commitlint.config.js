import base from './commitlint.base.js';

export default {
  ...base,
  rules: {
    ...base.rules,
    // 在此处扩展项目特有的 scope 白名单（例如 ['core', 'cli', 'app']）
    // 本文件受 update_ignore 保护，模板更新时不会被覆盖
  },
};