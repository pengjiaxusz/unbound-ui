# 自定义环境检查

`.pengj-templates/scripts/doctor.sh`（Unix）与 `.pengj-templates/scripts/doctor.ps1`（Windows）先检查所选模板层
实际用到的工具，再按文件名顺序运行本目录下的每个脚本。

## 契约

- Unix：`*.sh`，由 `sh` 执行。
- Windows：`*.ps1`，由 `pwsh -NoProfile -File` 执行。
- 工作目录：项目根目录。
- `DOCTOR_ROOT`：项目根目录。
- `DOCTOR_FIX=1`：doctor 以 `--fix` / `-Fix` 运行时设置。自定义检查可借此
  修复自己所依赖的工具。
- 退出码 `0` 表示通过；任何非 0 退出都会被记为失败，并使 doctor 以非 0 退出。

请让每个脚本自成一体。除非 `DOCTOR_FIX=1`，不要安装任何东西；只打印一行
简短说明，描述检查发现了什么。

## 示例（`.pengj-templates/scripts/doctor.d/10-cmake.sh`）

```sh
#!/bin/sh
if ! command -v cmake >/dev/null 2>&1; then
    echo "未找到 cmake；请从 https://cmake.org 安装"
    exit 1
fi
echo "cmake $(cmake --version | head -n 1)"
```