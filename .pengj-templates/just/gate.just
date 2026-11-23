# .pengj-templates/just/gate.just — 提交门禁配方（gate 层托管，模板更新整页覆盖）
# 提交门禁内置在 `pengj-templates-cli` 里，下面的检查配方都直接调 `pengj-templates-cli gate ...`。钩子激活走 `just doctor --fix`（或 `pengj-templates-cli gate hooks install`）——内置 doctor 是统一的激活入口。

# 对整个工作树跑代码语言守卫
gate-check:
    @pengj-templates-cli gate check

# 校验生成项目文案是否跟随语言轴：en 项目零汉字，zh 项目面向人的文案必须含中文。只检查 manifest 记录的模板托管文件。
gate-lang:
    @pengj-templates-cli check-lang --dir .

# 运行内置规则自检
gate-selftest:
    @pengj-templates-cli gate selftest