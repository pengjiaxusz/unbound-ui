# 01-graphics 图形底座

Vulkan 1.3 驱动的渲染底座：显存管理、绘制管线、着色策略与文字合成。

| 文档 | 内容 | 状态 |
| :--- | :--- | :--- |
| [核心 API 与特性开关](api-and-features.md) | Vulkan 1.3、BDA、Bindless、动态渲染 | `已落地` |
| [显存分层与并发安全](memory-hierarchy.md) | 持久池、瞬态环、离屏池、Epoch 回收 | `已落地` |
| [窗口呈现与零黑边](window-presentation.md) | DWM 对齐、present wait、延迟预算 | `部分落地` |
| [GPU 数据结构与 BDA 协议](gpu-data-layout.md) | Push Constants、图元实例、裁剪节点 | `已落地` |
| [Uber-Shader 与双通道深度](uber-shader.md) | 不透明内核预剔除 + 统一 uber pass | `已落地` |
| [双阶裁剪与局部坐标重构](clipping.md) | CPU 折叠 + VS 逆变换 + FS 无 discard | `已落地` |
| [解析解阴影](shadow.md) | erf 近似圆角矩形阴影积分 | `已落地` |
| [曲线与矢量渲染](curves.md) | 降阶细分、紧致 OBB、解析解 SDF | `已落地` |
| [工业级文字渲染](text.md) | 双图集、亚像素相位、GPU MTSDF | `已落地` |

渲染底座的代码全部集中在 `unbound-gpu`（约 8700 行）与 `unbound-vector`、
`unbound-text` 两个配套 crate。

---

**返回**：[文档索引](../README.md)
