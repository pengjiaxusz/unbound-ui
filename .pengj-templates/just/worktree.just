# .pengj-templates/just/worktree.just — 多工作树共享编译目录（worktree 层托管，模板更新整页覆盖）
# 共享目录就是**主工作树的 `target/`**：CLI 从 `git worktree list` 推导，同一仓库的所有工作树都得到同一个值；rust.just / rust-workspace.just 的构建配方也通过 `worktree target-dir` 读同一个值，推导逻辑只有一份。
# 代价：在主工作树里 `cargo clean`（或 `git clean -xdf`）会清掉所有工作树的中间产物，需全量重建。
# 共享的只有中间产物。最终二进制在共享目录里路径固定，多个工作树会互相覆盖——运行产物的隔离由项目自理。

# 打印共享编译目录（唯一来源：CLI 推导，配方与自检读同一个值）
wt-target:
    @pengj-templates-cli worktree target-dir --dir .

# 校验共享编译目录：存在、可写、没有被指向别处的覆盖值
wt-check:
    @pengj-templates-cli worktree check --dir .

# 修复模式：共享目录缺失时创建它（只补缺、不覆盖）
wt-fix:
    @pengj-templates-cli worktree check --dir . --fix

# 用法：just wt-create feature-a —— 建分支与同级目录、接线共享编译目录、补齐子模块
wt-create name:
    @pengj-templates-cli worktree create {{name}} --dir .