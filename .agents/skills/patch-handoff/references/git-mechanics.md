
# 落补丁集时的 Git 机制

## 一、钩子触发面矩阵

这是实测行为，不是文档传说。它决定哪道门禁还拦得住一条坏提交。

| 造成提交的方式 | `pre-commit` | `commit-msg` | `pre-applypatch` / `post-applypatch` | `pre-push` |
| :--- | :--- | :--- | :--- | :--- |
| `git commit` | 触发 | 触发 | 不触发 | 稍后 |
| `git commit --amend` | 触发 | 触发 | 不触发 | 稍后 |
| `git am` | **不触发** | **不触发** | 触发 | 稍后 |
| `git cherry-pick` | **不触发** | **不触发** | 不触发 | 稍后 |
| `git rebase` | **不触发** | **不触发** | 不触发 | 稍后 |

推论：

- 用 `git am` 落的序列，天然没经过项目的提交期检查（格式、语言守卫、消息规范）。唯一还拦得住的兜底是 `pre-push`。
- 别在 `pre-applypatch` 里修补：在钩子里写文件 + `git add` 进不了提交——`git am` 之后会 `read-tree` 冲掉索引，只留一棵脏工作树。这个钩子只能拦，不能改。
- 钩子存在 `.git/config`（`core.hooksPath`），是每个工作区自己的配置，不随仓库传播。在当前工作区确认已装钩子，否则 `pre-push` 等于不存在。（若用户明确要求隔离到克隆，则先在克隆里装。）
- 兜底真的生效的证据：推送输出里有钩子自己的日志行。没有日志行就是没钩子。

## 二、`git am` 失败诊断

按这个顺序往下走，不要一上来就改补丁。

1. **基线不对。** 对比目标 ref 的 `git log --oneline -3` 与文档声称的 SHA，在真实基线上重试。
2. **补丁已经应用过。** 报「patch does not apply」但 `git apply --check --reverse <patch>` 通过，确认内容已在树上再用 `git am --skip`。
3. **上下文漂移、内容没问题。** 用 `git am -3` 三方合并，落地后复查 diff。
4. **真冲突。** `git am --show-current-patch` 看它想要什么；解决后 `git add` + `git am --continue`；整体退出用 `git am --abort`。
5. **二进制或仅改模式的补丁。** 缺基线 blob，去要基线提交，不要手搓。
6. **邮箱格式损坏。** `.patch` 里缺 `From ` 分隔行或被 CRLF 弄坏，重新导出，不要修。

中途停下时把工作树留在原地——半应用状态就是证据——并在报告里写明。

## 三、把修复折回引入缺陷的提交

前提：该文件在整个序列中只被一条提交碰过。

```bash
git log --oneline <base>..HEAD -- <path> | wc -l    # 必须为 1
```

然后：用修好的版本覆盖文件、`git add`、在该提交仍是 `HEAD` 时 `git commit --amend --no-edit`——`scripts/replay-patch-series.sh` 做的就是这件事。若计数大于 1 就不要折叠：在末条提交单独修，并在报告里说明映射为何被破坏。

修复已经提交了的话，用 `git rebase -i --exec` 也可以，但绝不能让它弹编辑器（`GIT_SEQUENCE_EDITOR=true`）。

## 四、作者归一

```bash
# 目标身份由仓库自动解析：owner 取自 `origin` URL，邮箱取自 `git config user.email`。
# 绝不手填 name/email。
scripts/normalize-authors.sh <base> --print-identity   # 预览目标身份
scripts/normalize-authors.sh <base>                    # 重写作者、保留 author date
git log --format="%an <%ae>" <base>..HEAD | sort -u    # 必须恰好是目标那一行
```

- `--env-filter` 改作者、保留 author date；`--reset-author` 连时间一起重置，会毁掉交接文档描述的时间线。
- 脚本校验区间最终**恰好**是解析出的目标身份——只剩一个作者还不够，身份错了照样失败。
- committer 身份一般不动，除非项目另有约定；署名看的是 author 行。
- `filter-branch` 会重写区间内每一条提交，所以区间要小，且在推送之前做，绝不要推完再改。
- 只有在刻意覆盖时（如没有 `origin` remote）才位置传 `<name> <email>`，并在报告里写明理由。

## 五、强推之后

```bash
git fetch --prune --force <remote>        # 不加 --force，远端 ref 还是旧值
git rev-list --count <remote>/<branch>..HEAD   # 必须为 0，否则停手问用户
git status --porcelain -uall                   # 必须为空
git reset --hard <remote>/<branch>
```

强推后不要 `git pull`——它会把旧历史合回来；正确的动词是 `reset --hard`。若这个分支在父仓里是子模块指针，父仓会显示子模块已修改——那是指针差异，不是损坏，不要替用户提交或回滚父仓。

另外：共享构建缓存（`CARGO_TARGET_DIR`、放在公共位置的 `node_modules`）会让一次新克隆又把几十 GB 构建产物重新物化一遍——这也是就地干活、别重新克隆的理由之一。若被明确要求隔离到克隆，记得在 S8 清掉自己创建的东西。
