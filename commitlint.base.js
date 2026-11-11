const subjectLanguagePlugin = {
  rules: {
    // 要求提交标题必须用中文撰写：至少包含 2 个汉字，防止写成纯英文提交。
    'subject-language': ({ scope, subject, raw }) => {
      // 豁免：git revert 提交 或 release scope (如 main)
      if (raw && (raw.startsWith('revert:') || raw.startsWith('Revert "') || scope === 'main')) {
        return [true];
      }
      if (!subject) {
        return [false, '提交标题 (subject) 不能为空'];
      }

      const trimmedSubject = subject.trim();
      const chineseMatches = trimmedSubject.match(/[\p{Unified_Ideograph}]/gu) || [];
      const minChineseChars = 2;

      if (chineseMatches.length < minChineseChars) {
        return [
          false,
          `提交标题必须使用中文撰写（当前检测到汉字数: ${chineseMatches.length}，要求至少 ${minChineseChars} 个汉字）。\n` +
            `  错误示例: feat(cli): add new flags\n` +
            `  正确示例: feat(cli): 新增参数解析支持`,
        ];
      }
      return [true];
    },
  },
};

export default {
  extends: ['@commitlint/config-conventional'],
  plugins: [subjectLanguagePlugin],
  rules: {
    'body-max-line-length': [0], // 禁用正文行长度限制
    'subject-case': [0, 'always'], // 禁用 subject 大小写检查（允许中文标题）
    'type-enum': [
      2,
      'always',
      [
        'feat',
        'fix',
        'docs',
        'style',
        'refactor',
        'perf',
        'test',
        'build',
        'ci',
        'chore',
        'revert',
      ],
    ],
    'subject-language': [2, 'always'],
  },
};