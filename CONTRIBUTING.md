<!-- PENGJ_TEMPLATE_START -->
# unbound-ui 贡献指南

## 1. 克隆仓库

```sh
git clone https://github.com/pengjiaxusz/unbound-ui.git
```

## 2. 环境准备与自检

- 个人开发机（默认只读体检，不自动下载或修改工具）：
  ```sh
  # Linux / macOS
  sh .pengj-templates/scripts/doctor.sh
  # Windows
  .pengj-templates\scripts\doctor.cmd
  ```
- 新虚拟机 / CI / 自动化环境（一键装齐依赖并初始化，可先加 `--dry-run` 预览）：
  ```sh
  # Linux / macOS
  sh .pengj-templates/scripts/setup.sh
  # Windows
  .pengj-templates\scripts\setup.cmd
  ```
<!-- PENGJ_TEMPLATE_END -->

<!-- 项目专属贡献说明写在下方（块外，模板更新时 100% 保留）。 -->
