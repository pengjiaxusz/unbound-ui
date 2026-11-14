# Unbound UI 开发者入门与实战指南 (User Guide)

Unbound UI 是一个专为 Windows 10（1809+）和 Windows 11（21H2+）打造的现代、数据驱动（DOD）、GPU 直驱原生桌面 UI 框架。

它具有如下核心架构特性：
- **Flat POD 纯数据描述符**：零虚拟函数开销、零每帧堆内存分配（Zero Heap Allocation per frame）。
- **Stable Shape 契约**：所有组件在 24 种 `ControlState` 下保持图元数量恒定，支持 GPU 图元流式更新。
- **4 KiB Color LUT 零重传主题切换**：切换深色/浅色/高对比度主题无需重建图元缓冲，仅需更新 4 KiB 颜色索引查找表。
- **声明式人体工学布局**：基于 Taffy Flexbox 与 `ViewBuilder`，告别手动计算坐标 `Rect` 与文本字形排版。
- **开箱即用的原生桌面集成**：自定义标题栏保留 Windows 11 Snap Layouts（贴靠分屏）、CJK 输入法（IME）预编辑对齐、高分屏 Per-Monitor DPI v2 自动缩放。

---

## 目录
1. [快速上手 (Quick Start)](#1-快速上手-quick-start)
2. [数据驱动与免 Rect 布局心智模型](#2-数据驱动与免-rect-布局心智模型)
3. [声明式视图构建 (`ViewBuilder`)](#3-声明式视图构建-viewbuilder)
4. [组件状态与双向事件归约 (`reduce`)](#4-组件状态与双向事件归约-reduce)
5. [主题系统与 4 KiB Color LUT](#5-主题系统与-4-kib-color-lut)
6. [10 大标准渐进式示例导航](#6-10-大标准渐进式示例导航)
7. [无头环境与 AI Agent 自动化测试](#7-无头环境与-ai-agent-自动化测试)
8. [Windows 10/11 原生深度特性](#8-windows-1011-原生深度特性)

---

## 1. 快速上手 (Quick Start)

在 `Cargo.toml` 中引入 `unbound-app`：
```toml
[dependencies]
unbound-app = { path = "crates/unbound-app" }
unbound-math = { path = "crates/unbound-math" }
```

编写极简计数器应用（50 行内直接启动硬件加速窗口）：

```rust
use unbound_app::prelude::*;

pub struct CounterApp;

pub struct CounterState {
    pub count: i64,
}

impl Application for CounterApp {
    type State = CounterState;

    fn init(&mut self, _ctx: &mut AppContext) -> Self::State {
        CounterState { count: 0 }
    }

    fn reduce(&mut self, state: &mut Self::State, event: &AppEvent, _ctx: &mut AppContext) {
        match event {
            AppEvent::Click(key) if *key == ui_key!("btn_inc") => {
                state.count += 1;
            }
            AppEvent::Click(key) if *key == ui_key!("btn_dec") => {
                state.count -= 1;
            }
            _ => {}
        }
    }

    fn view(&self, state: &Self::State, ui: &mut ViewBuilder) {
        ui.column(ui_key!("root"), |col| {
            col.title_bar("My App");
            col.badge(
                ui_key!("counter_display"),
                BadgeDesc::label(format!("Count: {}", state.count)),
            );

            col.row(ui_key!("btn_row"), |row| {
                row.button(
                    ui_key!("btn_dec"),
                    ButtonDesc::label(" - Decrement ").variant(ButtonVariant::Secondary),
                );
                row.button(
                    ui_key!("btn_inc"),
                    ButtonDesc::label(" + Increment ").variant(ButtonVariant::Primary),
                );
            });
        });
    }
}

fn main() -> Result<(), PlatformError> {
    let config = WindowConfig {
        title: "Hello Counter - Unbound UI".to_string(),
        width: 480,
        height: 320,
        ..Default::default()
    };
    run_windowed(CounterApp, config)
}
```

运行方式：
```bash
just example 01_hello_counter
```

---

## 2. 数据驱动与免 Rect 布局心智模型

在传统即时模式（IMGUI）或保留模式（Retained）中，开发者常常需要手动传递坐标矩形 `Rect::new(x, y, w, h)` 或重载繁琐的布局属性。

Unbound UI 采用**两阶段声明式管道**：
1. **构造阶段**：组件使用免 `Rect` 的直观构造器与流式 Builder 描述：
   ```rust
   ButtonDesc::label("Save Document")
       .variant(ButtonVariant::Primary)
       .size(ButtonSize::Medium)
       .disabled(false);

   CheckboxDesc::labeled("Auto-save on exit", true);

   TextInputDesc::text(name)
       .placeholder("Enter username...")
       .clearable(true);

   SliderDesc::range(0.0, 100.0, current_volume).step(1.0);
   ```
2. **测量与排版阶段**：
   - 布局引擎调用 `UiWidget::intrinsic_size(&mut TextCtx)`，基于真实的字体排版度量（DirectWrite / ClearType 对齐）自动计算最佳宽高；
   - Taffy Flexbox 解算出全局绝对坐标 `node.world_rect`；
   - 发射阶段调用 `UiWidget::emit_laid_out`，自动根据真实边界生成字形图元 `GlyphQuad` 与图集增量上传。

### 编译期稳定 Key：`ui_key!`
所有挂载的组件均需通过 `ui_key!` 分配一个跨帧稳定的 `u64` 标识符：
- **静态节点**：`ui_key!("my_unique_name")`（编译期 FNV-1a 哈希，零运行时开销）；
- **动态循环节点**：`ui_key!("list_item", item_id)`（将基础哈希与动态索引/ID 混合）。

---

## 3. 声明式视图构建 (`ViewBuilder`)

`ViewBuilder` 提供了强大的流式嵌套容器与无边框窗口默认标题栏：

- `view.title_bar("标题")` / `view.title_bar_with(TitleBarDesc::new(...))`：为无边框窗口提供默认标题栏（包含拖拽区、Windows 11 贴靠悬停命中区、最小化、最大化/还原与关闭按钮），由 `WindowedRunner` 自动接管窗口控制与关闭流程。

### 基础布局容器
```rust
ui.column(ui_key!("col_layout"), |col| {
    col.row(ui_key!("row_layout"), |row| {
        row.button(ui_key!("btn_1"), ButtonDesc::label("One"));
        row.button(ui_key!("btn_2"), ButtonDesc::label("Two"));
    });
});
```

### 卡片与结构化分块 (`card`)
```rust
ui.card(ui_key!("settings_card"), CardDesc::default(), |card| {
    card.switch(ui_key!("dark_mode"), SwitchDesc::on(is_dark));
    card.slider(ui_key!("brightness"), SliderDesc::range(0.0, 100.0, 80.0));
});
```

### 虚拟滚动视图 (`scroll_view`)
```rust
ui.scroll_view(ui_key!("scroll_area"), ScrollAxis::Vertical, |view| {
    for idx in 0..1000 {
        view.badge(ui_key!("item", idx), BadgeDesc::label(format!("Row #{}", idx)));
    }
});
```

### 可拖拽停靠分割窗格 (`split_pane`)
```rust
ui.split_pane(
    ui_key!("editor_split"),
    SplitPaneDesc::new(Rect::ZERO, 0.3),
    |left| {
        left.badge(ui_key!("left_pane"), BadgeDesc::label("Navigation / Files"));
    },
    |right| {
        right.badge(ui_key!("right_pane"), BadgeDesc::label("Main Editor View"));
    },
);
```

### 挂载任意组件 (`add`)
`unbound-widgets` 中所有的 91 个标准组件变体（包括 `WidgetCatalogId` 中的所有组件）均实现了 `UiWidget` trait，均可一键挂载：
```rust
ui.add(ui_key!("progress_bar"), ProgressBarDesc::percent(0.75));
ui.add(ui_key!("data_table"), TableDesc::columns(my_columns));
```

---

## 4. 组件状态与双向事件归约 (`reduce`)

Unbound UI 将组件本身的瞬态局部状态（如输入框光标位置、下拉菜单开闭）与应用的业务状态优雅解耦：
- **`WidgetStateStore`**：运行时自动维护的局部状态池，生命周期与组件在视口中的挂载状态严格同步；
- **自动事件路由**：键盘输入、点击与指针拖拽首先被聚焦的组件拦截，自动推进内部光标或选中项，随后向应用派发语义级高阶事件；
- **状态查询**：在 `Application::reduce` 中，可通过 `ctx.state_store().get::<TextEditState>(KEY)` 轻松获取任何受控输入框的文本与选区。

```rust
impl Application for MyApp {
    type State = MyState;

    fn reduce(&mut self, state: &mut Self::State, event: &AppEvent, ctx: &mut AppContext) {
        match event {
            AppEvent::Click(key) if *key == ui_key!("submit_btn") => {
                if let Some(text_state) = ctx.state_store().get::<TextEditState>(ui_key!("input_field")) {
                    state.submitted_value = text_state.text().to_string();
                }
            }
            _ => {}
        }
    }
}
```

---

## 5. 主题系统与 4 KiB Color LUT

Unbound UI 的主题系统建立在 GPU 4 KiB Color LUT（Color Look-Up Table）之上：
- 图元顶点只存储颜色槽位索引（`Token` 枚举，共 256 槽位，每个槽位 16 字节 RGBA）；
- 主题切换时，无需重新录制或重传任何顶点缓冲；
- 调用 `ctx.theme = ThemeKind::Light`（或 `Dark`、`HighContrast`），下一帧仅向 Uniform Buffer 传输 4 KiB 字节，毫秒级实现整屏主题切换。

```rust
// 一键切换深浅色
ctx.theme = match ctx.theme {
    ThemeKind::Dark | ThemeKind::HighContrast => ThemeKind::Light,
    ThemeKind::Light => ThemeKind::Dark,
};
```

---

## 6. 10 大标准渐进式示例导航

仓库内置了 10 个标准可运行示例（位于 `crates/unbound-app/examples/`），每个示例均配备真实的无头自动化测试门禁：

| 示例文件 | 核心内容 | 快捷启动指令 |
| :--- | :--- | :--- |
| **`01_hello_counter.rs`** | 最小计数器、`ViewBuilder` 基础、主题一键切换 | `just example 01_hello_counter` |
| **`02_form_and_validation.rs`** | 真实表单录入、`TextInput`、`Select`、实时校验与错误提示 | `just example 02_form_and_validation` |
| **`03_data_table_and_virtual_list.rs`** | 100k 行极速虚拟滚动、`Table` 列排序与 `Pagination` 分页 | `just example 03_data_table_and_virtual_list` |
| **`04_docking_ide_workspace.rs`** | IDE 多窗格工作区、`SplitPane`、`CodeEditor` 与 `TreeView` | `just example 04_docking_ide_workspace` |
| **`05_overlays_and_navigation.rs`** | 模态框 `Modal`、`Drawer` 抽屉、焦点陷阱与 `Breadcrumb` 面包屑 | `just example 05_overlays_and_navigation` |
| **`06_infinite_canvas_node_editor.rs`** | 无限平移缩放画布、节点图 `NodeGraph`、曲线编辑器与历史撤销 | `just example 06_infinite_canvas_node_editor` |
| **`07_async_assets_and_i18n.rs`** | 后台异步通道、`set_timeout` 定时器、多语言 I18n 与 RTL 镜像 | `just example 07_async_assets_and_i18n` |
| **`08_hot_reload_guest_app.rs`** | Host-Guest C-ABI 毫秒级极速热重载模板与状态模式迁移 | `just example 08_hot_reload_guest_app` |
| **`09_headless_agent_testing.rs`** | AI Agent 自动化无头自省、5 项几何体检与时间旅行调试 | `just example 09_headless_agent_testing` |
| **`10_win11_native_desktop_shell.rs`** | Windows 11 自定义标题栏、Snap Layouts、文件拖拽与水印 | `just example 10_win11_native_desktop_shell` |

所有示例均已纳入 CI 门禁，一次性验证全部 10 个示例：
```bash
just test-examples
```

---

## 7. 无头环境与 AI Agent 自动化测试

无需物理 GPU、无需显示器，可在纯内存中模拟运行任意 `Application`：
```rust
#[test]
fn test_my_app_logic() {
    let mut runtime = AppRuntime::new(CounterApp, Vec2::new(800.0, 600.0));

    // 运行初始帧并检查生成的图元
    let report = runtime.frame(0.0);
    assert!(report.primitives > 0);

    // 查找组件的全局几何矩形
    let inc_btn = runtime.widget_rect(ui_key!("btn_inc")).expect("Button should be mounted");

    // 模拟鼠标点击
    runtime.pointer_move(inc_btn.center());
    let _ = runtime.pointer_down(inc_btn.center(), PointerButton::Left);
    let _ = runtime.pointer_up(inc_btn.center(), PointerButton::Left);

    // 驱动下一帧
    let _ = runtime.frame(1.0 / 60.0);

    // 断言状态正确演进
    assert_eq!(runtime.state().count, 1);
}
```

---

## 8. Windows 10/11 原生深度特性

- **Windows 11 Snap Layouts**：自定义无边框标题栏拦截 `WM_NCHITTEST`，悬停最大化按钮时无缝弹出 Windows 11 系统级贴靠分屏布局菜单。
- **CJK 输入法（IME）精确跟随**：预编辑文本（Preedit）和光标选区自动同步，候选词窗口严密贴合当前聚焦输入控件的光标位置。
- **Per-Monitor DPI v2**：窗口在不同缩放比（100%、125%、150%、200%）显示器之间拖拽时，字形图集与几何尺寸平滑无缝重构，彻底杜绝模糊。
- **文件拖放 (`FileDrop`)**：无缝接收从 Windows 资源管理器（File Explorer）拖入的文件路径列表。

---

## 9. 质量与验证规范

每次代码提交前，请务必执行以下验证流程：
```bash
# 格式化代码
just fmt

# 全量编译与 Clippy 零警告检查
just check

# 运行 10 大示例自动化测试
just test-examples

# 运行工作区完整测试套件
just test
```
