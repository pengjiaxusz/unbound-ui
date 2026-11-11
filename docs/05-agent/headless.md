# 无头闭环驱动验证

> **实现状态**：`已落地`
>
> | | |
> | :--- | :--- |
> | **源码** | `crates/unbound-headless/src/{frame,rules,snapshot,driver}.rs` |
> | **关键类型** | `Harness`、`TestApp`、`Frame`、`FrameBuilder`、`Rule`、`Diagnostic`、`FrameSnapshot`、`TimeTravel` |
> | **实测** | 无头单帧验证 544 µs（预算 1.0 ms） |

## 亚毫秒级纯内存驱动环境

完全脱离操作系统窗口句柄（HWND）与 Vulkan GPU 实例，在纯 CPU 内存中实例化
`UiWorld` 与无限画布空间索引。

在 **小于 $1.0\text{ms}$** 内完成完整闭环：

```
模拟输入事件序列派发 → Reducer 状态演进 → Taffy 局部增量排版
    → 松散四叉树同步 → 图元生成 → 自动几何体检
```

> **当前边界**：本 crate 只对**已求解**的几何负责。`Frame` 里的矩形由调用方
> （组件发射器或展台）给出绝对坐标后通过 `FrameBuilder` 录入，Taffy 增量排版
> 尚未接入无头路径；快照、五项体检与交互驱动都不依赖 `unbound-layout`。

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

## 无头交互测试框架（`TestApp`）

`Harness` 回答的是「这一帧的几何是否合法」，它不知道指针、焦点和语义树。
可交互控件还需要另一件事：**让一次点击走完整条输入链路**——命中测试、指针
捕获、点击合成、焦点迁移、语义树更新。`driver::TestApp` 就是为此提供的会话
对象，纯 CPU、无 GPU、无窗口句柄。

### 会话拥有什么

| 成员 | 作用 |
| :--- | :--- |
| `world: UiWorld` | 控件身份 `NodeId` 的分配处 |
| `router: Router` | 热区、hover、指针捕获与拖拽 |
| `semantics: Semantics` | AccessKit 增量语义树 |
| `viewport` / `zoom` / `pan` | 与窗口完全相同的相机参数 |
| 私有 `focus` / `pointer` / `time` / `generation` | 焦点槽位、指针位置、逻辑时钟、场景代数 |

### 输入一律用窗口像素

测试写的是用户看得见的坐标，驱动内部做与窗口相同的逆变换，所以缩放、平移之后
的点击位置仍然正确：

```text
screen = (canvas - anchor) * zoom + viewport / 2
canvas = (screen - viewport / 2) / zoom + anchor
```

`screen_to_canvas` / `canvas_to_screen` 是这对映射的公开入口；`pick`、`move_to`、
`pointer_down`、`pointer_up`、`click`、`drag` 全部以窗口像素为参数。`zoom_at`
保持锚点下的画布点不动，`pan_by` 使用窗口像素增量。

### 合成输入

| 方法 | 行为 |
| :--- | :--- |
| `move_to(screen)` | 移动指针并更新 hover，返回命中的节点 |
| `pointer_down()` / `pointer_up()` | 按下 / 松开，返回捕获节点 / 事件序列 |
| `click(screen)` | 移动 + 按下 + 松开，返回 `ClickReport` |
| `drag(from, to)` | 按住从 `from` 拖到 `to` 再松开 |
| `cancel()` | 模拟窗口失焦，丢弃捕获并发出 `CaptureLost` |
| `pick(screen)` / `cursor()` | 不改指针状态地查询命中与光标 |

`ClickReport` 给出 `pressed`（按下时捕获的节点）、`events`（路由器原样事件），
以及 `clicked()` / `click_count()` / `is_double_click()` 三个断言快捷方式。
`advance(seconds)` 推进逻辑时钟，用来把两次点击排除在双击窗口之外。

指针捕获由路由器保证：按下后拖出热区仍然保持 pressed，在热区外松开**不会**
合成 `Click`——这与原生按钮一致，也是旧版用 `bool` 记录状态时最容易失步的地方。

### 提交场景：`install` 与 `generation`

`install(regions, semantics)` 是「这一帧场景已重建」的唯一入口：替换热区、替换
语义树、把焦点重新应用到新树、并递增 `generation`。控件只需在离散状态
（视觉态、点击计数）变化时重建场景并调用它，测试就能用 `generation` 判断是否
真的发生了重建——GPU 像素测试正是靠每次重建拿到的新 `generation`，让渲染器
重新上传 idle / hover / pressed 三种状态的图元。

### 焦点与语义断言

焦点以语义 id（`u64`）存放，`set_focus(Option<NodeId>)`、`set_focus_key(raw)`、
`focused_key()` 三条路径共享同一个槽位，`apply_focus` 把它镜像进 `Semantics`。
`semantics_node`、`assert_label`、`assert_value`、`assert_focus` 直接对语义树断言。

> **为什么焦点存 `u64` 而不是 `NodeId`**：`slotmap` 的 `KeyData` 会重编码版本位，
> `NodeId::from(KeyData::from_ffi(7)).as_ffi()` 并不等于 `7`。用 `NodeId` 做往返
> 会让「按原始 id 安装的语义树」找不到焦点。`node_key(node)` 是唯一的转换入口。

### 两层结构：驱动在引擎，薄壳在控件

`TestApp` 刻意不认识任何具体控件，也不认识渲染与布局。控件侧只写一层薄壳，
把「身份 + 文案 + 计数 + 最近一次场景」拼起来，例如
`apps/showcase/src/testing.rs` 的 `ButtonHarness`：它从路由器派生
`ButtonVisualState`，只在 `(视觉态, 计数)` 变化时重建场景，然后把热区与语义树
交给 `install`。

```rust
let mut app = TestApp::new((1280.0, 720.0));
app.install(vec![region], semantics);

let report = app.click(Vec2::new(200.0, 120.0));
assert_eq!(report.clicked(), Some(node_key));
app.assert_label(node, "Clicked: 1");
app.assert_focus(Some(node));
```

五项体检也从驱动直达：`app.check(&frame)` / `app.assert_clean(&frame)`。因此一个
控件测试可以在同一次会话里先点击、再断言语义、最后跑几何体检。

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

**相关文档**：[无障碍语义](../04-input/accessibility.md) · [组件展台](../06-tooling/showcase.md) · [热重载](../06-tooling/hot-reload.md)

**返回**：[05-agent](README.md) · [文档索引](../README.md)
