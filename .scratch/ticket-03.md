## Parent

Part of #74

## What to build

将用例指纹增量过滤与装箱排版打通，已全部缓存的控件完全不进入任何拼版页，全部命中时毫秒级跳过；落定 `docs/adr/0015-dynamic-multi-control-parity-pages.md` 决策记录；完成与 Rust 端 `showcase --parity-run` 的双端差分回归与交接文档同步。

保证开发人员在未修改控件规格时享受零等待缓存跳过，同时确保生成的切片产物完全兼容现有 CIEDE2000 四重门禁。

## Acceptance criteria

- [ ] 在装箱前基于用例指纹（SDK 版本、系统构建号、用例规格等）对控件进行过滤；只有当控件内存在至少一个未命中状态或被显式指定时，才将其送入装箱规划器
- [ ] 若某主题下的所有控件均已命中缓存，系统直接打印 `[cache-hit]` 并跳过当前主题所有拼版页的渲染与等待，达到毫秒级退出
- [ ] 撰写并落定 `docs/adr/0015-dynamic-multi-control-parity-pages.md`，记录解耦瞬态采集载体与切片独立产物的架构权衡
- [ ] 运行 `cargo run -p showcase -- --parity-run --reference <output_dir>`，验证 Rust 端离屏差分器能无缝消费切片单图，四重门禁全绿并通过回归测试
- [ ] 更新 `tools/winui-reference/README.md` 与 `docs/agents/handoff-winui3-parity.md`，同步最新的动态拼版采集命令与参数说明

## Blocked by

- #76
