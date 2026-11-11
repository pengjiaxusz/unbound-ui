# 无头闭环驱动验证

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-headless/src/{frame,rules,snapshot}.rs` |
> | **关键类型** | `Harness`、`Frame`、`FrameBuilder`、`Rule`、`Diagnostic`、`FrameSnapshot`、`TimeTravel` |
> | **实测** | 无头单帧验证 544 µs（预算 1.0 ms） |

## 亚毫秒级纯内存驱动环境

完全脱离操作系统窗口句柄（HWND）与 Vulkan GPU 实例，在纯 CPU 内存中实例化
`UiWorld` 与无限画布空间索引。

在 **小于 $1.0\text{ms}$** 内完成完整闭环：

```
模拟输入事件序列派发 → Reducer 状态演进 → Taffy 局部增量排版
    → 松散四叉树同步 → 图元生成 → 自动几何体检
```

## 五项自动化几何体检断言

| 规则 | 检查内容 |
| :--- | :--- |
| `AssertNoNaNOrInf` | 拦截任何排版或坐标变换计算得出的 `NaN`、`Infinity` 或负数宽高异常 |
| `AssertTextNotClipped` | 结合塑形结果，检测文本真实墨水包围盒（Ink Bounding Box）是否在未声明 `text-overflow: ellipsis` 或滚动容器的情况下被父容器裁剪截断 |
| `AssertInteractiveTargetSize` | 检查所有绑定了交互 `Action` 的可点击组件，其最终裁剪后的有效拾取屏幕面积是否小于最小可交互阈值（默认 $24 \times 24\text{px}$），或中心点是否被非预期的兄弟节点完全遮挡 |
| `AssertNoUnintendedSiblingOverlap` | 检测同一 Flex/Grid 布局容器内的同级子元素是否由于约束冲突发生非预期的几何包围盒重叠 |
| `AssertViewportContainment` | 验证所有通过 `Portal` 弹出的菜单、对话框与 Tooltip 是否 $100\%$ 处于物理视口边界内部 |

`Rule::ALL` 是这五条规则的常量数组，`check(frame)` 返回全部违规项的
`Vec<Diagnostic>`。

## 精准源码定位与轻量二进制快照

所有组件构建函数自动携带 Rust `#[track_caller]` 属性，记录创建该节点的源码
文件名与行号（`Location::caller()`）。

一旦几何体检断言触发，立即输出包含 `NodeId`、源码精确行号、父子约束链、实际
计算出的 AABB 以及 `AccessKit` 语义路径的**结构化 JSON 错误栈**，供 Agent 直接
精准修复代码。

`Diagnostic::to_json()` 生成该错误栈。

### 时间旅行调试

状态回滚采用零拷贝二进制序列化（`rkyv`）：

| 操作 | 接口 | 说明 |
| :--- | :--- | :--- |
| 归档 | `capture(frame)` | 生成 `FrameSnapshot` |
| 落盘 | `open(path)` | 写入二进制文件 |
| 就地读取 | `snapshot.access()` | 返回 `&Archived<FrameSnapshot>`，零拷贝 |
| 恢复 | `restore(snapshot)` | 还原帧状态 |

时间旅行快照采用 rkyv 的原因很直接：`postcard` 需要反序列化才能访问，而
rkyv 的 `access` 直接给出归档视图的引用——回滚到任意历史帧不付反序列化代价。

### 快照里的节点 id 不可信

快照要能过网络，所以帧里的 id 是**发送方说了算**的，而不是本地构建器分配的
连续序列。恢复路径因此必须能容忍任意 id：

| id 形态 | 曾经的结局 | 现在 |
| :--- | :--- | :--- |
| `0` | 索引按 `id - 1` 定位，减一在零上**溢出 panic** | 回退到有序 `(id, position)` 表 |
| `u64::MAX` | 索引**按 id 本身分配**，直接 capacity overflow | 同上，不按 id 分配 |
| 重复 id | 两个节点别名到同一槽位 | 同样回退，查找不再混淆两者 |

密集 id（构建器产出的那种）仍然走 `Vec` 快路径——体检器在百万次调用里查节点，
这条路径不能退化成二分。

---

**相关文档**：[无障碍语义](../04-input/accessibility.md) · [热重载](../06-tooling/hot-reload.md)

**返回**：[05-agent](README.md) · [文档索引](../README.md)
