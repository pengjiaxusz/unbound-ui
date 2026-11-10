# 提交规范

本仓库统一采用 [Conventional Commits](https://www.conventionalcommits.org/) 约定式提交。

## 格式

```
<type>[optional scope][!]: <description>

[optional body]

[optional footer(s)]
```

- **type**（必填）：`feat` | `fix` | `docs` | `style` | `refactor` | `perf` | `test` | `build` | `ci` | `chore` | `revert`
- **scope**（可选）：受影响模块，如 `renderer`、`layout`、`canvas`，使用小写字母/数字/`.` `_` `-` `/`
- **!**（可选）：表示破坏性变更（BREAKING CHANGE），此时 footer 中应写明影响与迁移方式
- **description**（必填）：一句话说明「做了什么」，用祈使句/现在时，首字母小写，结尾不加句号；中英文均可，建议不超过 72 字符
- **body / footer**（可选）：补充说明动机、影响范围，或 `Refs: #123`、`BREAKING CHANGE: ...`

## 常用 type

| type | 用途 |
| :--- | :--- |
| `feat` | 新增功能 |
| `fix` | 修复缺陷 |
| `docs` | 文档变更 |
| `style` | 格式调整，不影响逻辑 |
| `refactor` | 重构，既不新增功能也不修复缺陷 |
| `perf` | 性能优化 |
| `test` | 增补或修改测试 |
| `build` | 构建系统、依赖变更 |
| `ci` | CI 配置与脚本 |
| `chore` | 其他杂项维护 |
| `revert` | 回滚既有提交 |

## 示例

```
docs: 添加 GPU 驱动 UI 与无限画布引擎架构蓝图 V2.0
feat(renderer): 实现双通道 Uber-Shader 预剔除
perf(canvas): 分块相对坐标解算消除每帧重传
fix(layout): 修复旋转图元裁剪时的顶点畸变
refactor(core)!: 将节点树改为 SlotMap 扁平存储

BREAKING CHANGE: NodeId 不再可解引用为指针
```

## 本地校验

仓库内置零依赖的 `commit-msg` 钩子，会在提交时自动校验首行格式：

```bash
git config core.hooksPath .githooks
```

合并提交（`Merge ...`）与 `git revert` 自动生成的提交信息不受校验。
