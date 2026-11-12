# 无障碍语义体系

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-a11y/src/lib.rs`、`crates/unbound-platform/src/a11y_bridge.rs`、`crates/unbound-platform/src/a11y_winit.rs` |
> | **关键类型** | `Semantics`、`A11yNode`、`A11yAction`、`A11yActions`、`A11yBridge`、`A11yRequest`、`WinitA11y` |
> | **依赖** | `accesskit`、`accesskit_winit` crate |

## 语义树作为一级输出端

将 `AccessKit` 作为 UI 框架的一级语义输出端，与 `SlotMap` 节点生命周期和增量
脏标记深度绑定。

仅在节点发生状态或布局变更时，增量构建 `accesskit::TreeUpdate` 推送给操作系统
无障碍接口（Windows UI Automation / macOS NSAccessibility / Linux AT-SPI）。

## 双模赋能

同一棵语义树同时服务于两类消费者：

```
                  ┌──────────────────────────┐
                  │      Semantics 树         │
                  │   (A11yNode 层次结构)      │
                  └───────────┬──────────────┘
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
   ┌────────────────────┐         ┌────────────────────┐
   │ 操作系统无障碍接口   │         │   AI Agent 自省      │
   │ (UIA / NSAccessibility│        │ (首选零成本协议)      │
   │  / AT-SPI)          │         │                    │
   └────────────────────┘         └────────────────────┘
```

同时将该语义树直接暴露为 **AI Agent 的首选零成本自省协议**，无需维护第二套
自定义 AST。

这是[Agent 友好设计](../05-agent/README.md)的基石：Agent 不需要理解渲染树，
只需要读语义树。

## 节点与动作

| 类型 | 职责 |
| :--- | :--- |
| `A11yNode` | 单个语义节点：角色、标签、边界、可选动作集 |
| `A11yActions` | 动作位集合（16 位），支持插入/移除/查询 |
| `Semantics` | 语义树的根，持有根节点 ID 与节点表 |

`A11yNode::to_accesskit` 把内部表示转换为 `accesskit::Node`，其中边界使用
`accesskit::Rect::from_origin_size` 转换——语义树直接使用物理屏幕坐标。

## 平台桥接

`A11yBridge` 负责语义树与平台适配器之间的双向边界：

- **出站**：`Semantics::take_update` 产出的 `TreeUpdate` 交给实现了
  `A11yTransport` 的适配器；无障碍关闭时 `publish` 直接返回，不产生任何开销。
- **入站**：平台回传的 `ActionRequest` 由 `translate` 映射为 `A11yRequest`
  （节点 id + `A11yAction` + 可选新值），再进入应用 `reduce`，与指针、键盘事件
  走同一条处理路径。

`unbound-platform` 只定义传输契约，不绑定具体窗口库，因此桥接可以在无头环境中
用记录型 transport 完整测试。窗口壳层由 `WinitA11y`（`accesskit_winit::Adapter`）
完成实机接入，每个窗口一份：

- 适配器在窗口 **显示之前** 创建（AccessKit 的硬性要求），由
  `PlatformApp::window_created` 之后统一托管。
- 每帧由平台调用 `PlatformApp::accessibility_update` 拉取增量，
  `WinitA11y::publish_with` 只在有辅助技术监听时消费该增量；静态帧不产生
  任何系统调用。
- OS 回传的 `ActionRequest` 进入通道，事件循环在帧边界取出并交给
  `PlatformApp::accessibility_action`，再进入 `AppRuntime::a11y`，与指针、键盘
  事件共用同一条归约路径。

## 边界同步

节点边界来自布局结果，因此语义树与视觉呈现**永远一致**——不存在"语义树说按钮
在左边，视觉上在右边"的可能。这是增量推送能成立的前提：只有边界变化才需要
推送更新。

---

**相关文档**：[无头闭环验证](../05-agent/headless.md) · [面向数据拓扑](../02-ui/dod-model.md)

**返回**：[04-input](README.md) · [文档索引](../README.md)
