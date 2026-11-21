---
name: write-just
description: >-

  编写与审查根 justfile 与 `.pengj-templates/just/*.just` 层模块，让重型命令始终接入 bque 排队：规范产物位置、Windows/Unix 成对配方，以及 cargo run/build/clippy/test/bench 的强制包装。当需要新增或修改 just 配方、给某个层接 just 模块，或在 justfile 里看到裸跑 `cargo run` / `cargo build` 时使用。
  Triggers: justfile, just 配方, .just, bque, q-run, cargo run, PENGJ_TEMPLATE_START, [windows], [unix], import, 层模块.

---

<!-- PENGJ_TEMPLATE_START -->

# write-just — justfile 与层配方编写

项目的命令入口只放在两处：唯一的根 `justfile` 与 `.pengj-templates/just/` 下的托管模块。Windows 上任何重型命令都要接入 bque 排队。本技能把新配方约束在这个边界内——最常见的翻车就是手写一句 `cargo run`，悄悄绕过了队列。

## 1. 配方写在哪

| 你要写的东西 | 落点 | 归属 |
| :--- | :--- | :--- |
| 入口、`set shell`、`default`、条件 `import` | 根 `justfile` 的 `# PENGJ_TEMPLATE_START/END` 内 | `just` 层，`update` 原位替换 |
| 项目专属配方 | 根 `justfile` 的托管块**外** | 项目所有，永不覆盖 |
| 勾某层就要下发的配方 | `.pengj-templates/just/<层>.just` | 该层所有，`update` 整页覆盖 |

不要在根下新建 `scripts/` 或 `just/`——这两个名字留给项目自己。生成项目里不要手改托管 `.just` 模块；改模板后重跑 `update`。

## 2. 四条硬规则

1. **只要 `layers` 含 `bque`，重型命令一律包装**——包括 `cargo run`。用 `bque run -- <命令>`，跑分用 `bque bench -- <命令>`。裸写 `cargo run/build/clippy/test/bench`、`ninja`、`cmake --build`、`ctest`、`pnpm build` 都算缺陷。
2. **没有 bque 层？** 直跑是当前层的合法降级——在注释里说明，别把它包装成推荐终点。严禁在 Unix / CI 里强行套原生 bque。
3. **平台成对**：Windows 与 Unix 行为不同就同时写 `[windows]` 与 `[unix]` 两个配方。bque 是 Windows 全局锁，`[unix]` 分支直跑透传属预期，不算绕过。
4. **变参**：声明 `*ARGS` 并用 `{{ARGS}}` 转发。临时重命令优先 `just q-run <命令>` / `just q-bench <命令>`（配方名后不要加 `--`）。

## 3. 最小正确配方

```just
# 运行程序（Windows 走全局排队 + Idle 优先级）
[windows]
run *ARGS:
    @bque run -- cargo run {{ARGS}}
[unix]
run *ARGS:
    cargo run {{ARGS}}
```

反例——不要这样写：

```just
run *ARGS:
    cargo run {{ARGS}}   # 在 Windows 上绕过了 bque
```

## 4. 改模板里的 `.just` 文件

面向人的注释必须同时写中英两支（`{% if options["skill_lang"] | default("zh") == "en" %} … `{% else %} … `{% endif %}`），且渲染出的中文结果要与改前逐字节一致。配方里的字面变量（如 `{{ARGS}}`）必须用 minijinja raw 块包住——照抄上面示例或任一现有 `.just` 模板的写法。`import` 按 `layers` 条件开关；模块头部注明归属层。

完整命令映射、更多坑与无 bque 时的降级写法：[references/command-map.md](references/command-map.md)。

## 5. 自检

```sh
sh .agents/skills/write-just/scripts/check-just.sh          # 扫描 justfile + 托管模块
sh .agents/skills/write-just/scripts/check-just.sh --dir .  # 显式指定项目根
```

脚本只读，发现未包装的重型命令时返回非零退出码。修完再收尾——这是真缺陷，不是风格提醒。


<!-- PENGJ_TEMPLATE_END -->


<!-- 项目专属区域：模板更新只替换上方托管块，本区域归项目所有。 -->
## 项目专属配方约定

> 在此登记项目特有的耗时命令、自定义排队策略，或有意不包装的配方及理由。
