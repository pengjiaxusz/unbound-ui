
# 子仓库升级与宿主代码适配实战范例 (EXAMPLES.md)

本文档归纳真实的子仓库升级场景，展示从提交差异提取、聚类影响面分析，到宿主代码适配与重构的完整工作流。

---

## 范例一：探测未应用提交与多维影响面聚类

在决定升级子仓库前，首先运行探测脚本，获取差异画像：

```powershell
pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath "submodules/shared-ui"
```

**控制台输出示例**：
```text
=================================================================
             shared-ui 子仓库未应用提交比对分析               
=================================================================
【当前老位置】: a1b2c3d (a1b2c3d4e5f6...)
               └─ fix(dropdown): prevent click propagation outside modal
【目标新位置】: 7e8f9a0 (7e8f9a012345...) [来源: ..\shared-ui -> HEAD]
               └─ feat(code-block): support theme-aware syntax highlighting
-----------------------------------------------------------------
发现未应用的提交记录: 18 条

【涉及系统与影响面聚类】
  💥 破坏性变更 (Breaking Changes) [1 项] - 需重点核验:
     - 4f2a1b9 feat(button)!: rename variant 'danger' to 'destructive'

  🎬 动效与过渡系统 (Motion) [3 项]:
     - b9c8d7e feat(motion): add ease-standard and ease-decelerate tokens
     - 3a4b5c6 fix(dialog): resolve exit transition flickers

  🧩 规范控件与组件 API 变更 (Components) [5 项]:
     - 7e8f9a0 feat(code-block): support theme-aware syntax highlighting
     - 1a2b3c4 feat(duration-input): add duration input widget with unit selector

-----------------------------------------------------------------
【完整未应用提交列表 (git log oneline)】:
  7e8f9a0 feat(code-block): support theme-aware syntax highlighting
  4f2a1b9 feat(button)!: rename variant 'danger' to 'destructive'
  ...
=================================================================

💡 提示：若需一键将子仓库签出更新至上述最新提交，可运行：
  pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath 'submodules/shared-ui' -Update
```

---

## 范例二：根据聚类结果重构替换宿主自研轮子

**背景**：
在聚类报告中发现 upstream 新增了 `<DurationInput>` 规范控件。而宿主项目中原先手写了 200+ 行输入控制与单位换算逻辑：

### 重构前（宿主自研手写）
```tsx
// 宿主业务层手写的临时实现：冗长且难维护
export function TimeDurationEditor({ valueMs, onChange }: Props) {
  const [unit, setUnit] = useState('ms');
  const [val, setVal] = useState(valueMs);

  const handleUnitChange = (u: string) => {
    // 各种手动倍率计算...
  };

  return (
    <div className="flex gap-1">
      <input type="number" value={val} onChange={...} />
      <select value={unit} onChange={...}>...</select>
    </div>
  );
}
```

### 重构后（优雅接入上游规范控件）
```tsx
import { DurationInput } from "@org/shared-ui";

export function TimeDurationEditor({ valueMs, onChange }: Props) {
  return (
    <DurationInput
      valueMicroseconds={valueMs * 1000}
      onChangeMicroseconds={(us) => onChange(us ? us / 1000 : 0)}
      preset="standard"
    />
  );
}
```

**效果**：
- 删除了 200+ 行重复样板代码；
- 统一了产品级交互手感与键盘快捷操作；
- 遵循了宿主架构中的「组件自举律」。

---

## 范例三：CMake / FetchContent 依赖升级与重构流程

对于基于 CMake FetchContent 或缓存目录维护的子仓库（例如 `target/debug/_deps/mylib-src`）：

```powershell
# 1. 探测未应用提交
pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath "target/debug/_deps/mylib-src"

# 2. 执行更新并重新配置 CMake
pwsh .agents/skills/subrepo-sync/scripts/show-unapplied-commits.ps1 -SubrepoPath "target/debug/_deps/mylib-src" -Update
cmake -B target/debug -S .

# 3. 执行宿主门禁验证
ctest --test-dir target/debug --output-on-failure
```

---

## 范例四：完成升级后的规范提交

适配完成并通过本地门禁后，撰写符合 Conventional Commits 的提交信息：

```bash
git add submodules/shared-ui src/components/TimeDurationEditor.tsx
git commit -m "chore(deps): 更新 shared-ui 至 7e8f9a0 并接入规范 DurationInput 组件"
git push
```
