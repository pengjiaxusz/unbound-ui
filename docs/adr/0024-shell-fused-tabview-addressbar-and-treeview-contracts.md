# ADR-0024：壳层融合 TabView、双模 AddressBar 与虚拟化导航 TreeView 契约

- **状态**：已采纳
- **影响 crate**：`unbound-widgets`、`unbound-world`、`unbound-app`、`unbound-theme`

## 背景

桌面文件管理器与多标签工作台需要三类高度协同的 Fluent（WinUI 3）导航控件：可融入无边框标题栏的卡片式 `TabView`、支持面包屑与原位路径编辑的双模 `AddressBar`，以及支持懒加载与多挂载点隔离的虚拟化 `TreeView`。

当前框架内三者存在明显能力落差：`tabs.rs` 仅提供矩形底部指示条形态，不支持标题栏融合卡片与自适应宽度；`address_bar.rs` 仅为 198 行静态展示骨架，缺少独立命中区、图标与真实编辑态；`tree_view.rs` 缺少行图标槽、懒加载契约与出现键隔离。同时，根据术语表中的**基准形态对齐**原则，不得在 WinUI 3 基准对齐控件内塞入非标外壳逻辑而污染像素差分。

## 决策

1. **守住基准形态对齐边界**：
   - `TitleBar`、`TabView`、`BreadcrumbBar` 与 `TreeView` 继续作为 WinUI 3 基准对齐控件，默认形态严格对齐官方模板并参与批量像素差分；
   - `AddressBar` 作为自有独有复合控件在 `address_bar.rs` 内完整重写，通过组合 `IconButton`、`BreadcrumbBar`、`编辑核心`（`TextEditState`）、`MenuFlyout` 与 `AutoSuggestBox` 遵循 Fluent 设计规律，不污染基准差分。
2. **标题栏与标签栏以官方协作原语融合，不新建打包窗口控件**：
   - `TabView`（`tabs.rs`）新增 `TabViewStyle::ShellChrome` 卡片融合形态（顶部圆角、底边与下方工具栏表面无缝衔接、非选中标签间细竖线、图标 + 截断标题 + 独立关闭按钮、`+` 新增按钮、自适应宽度 `100..=240px` 与溢出滚动）；
   - `TitleBar`（`title_bar.rs`）扩展左侧内容槽与自适应拖拽区计算（始终保留不小于 `TitleBarMinDragRegionWidth = 48px` 的可拖拽空白区与右侧 `46×3` 窗口控制按钮区），实现类似 WinUI 3 `ExtendsContentIntoTitleBar` + `TabView` 的官方协作模式。
3. **分阶段落地交互深度（Phase 1 / Phase 2）**：
   - **Phase 1（本阶段必达）**：
     - `TabView`：Win11 卡片视觉、标题栏融合、选中/悬停/关闭 `×`、中键关闭、`+` 新增事件与溢出滚动；
     - `AddressBar`：外部独立 `← → ↑ ↻` 微妙图标按钮、一体化地址框、面包屑分段点击、`›` 分隔符触发同级子目录 `MenuFlyout`、点击空白或快捷键原位切入 `编辑核心` 文本编辑，以及右侧可折叠内联搜索框；
     - `TreeView`：行图标槽（`TreeRowIcon`，同时支持框架矢量图标与外部纹理句柄变体）、紧凑行高档位（28px 导航窗格密度，默认保持 WinUI 36px）、懒加载三态（`has_children / is_loaded / is_loading`）与 `ExpandRequested` 事件、以及 `OccurrenceKey` 与逻辑实体 ID 分离。
   - **Phase 2（后续迭代）**：
     - `TabView` 鼠标拖拽重排与跨窗撕出交互衔接（复用 ADR-0023 多窗口原语）；
     - `AddressBar` 文本编辑态路径历史与自动补全建议弹层；
     - `TreeView` 多级冻结祖先（`sticky_items`）与树节点拖拽落点反馈。

4. **Windows 11 资源管理器布局、裁剪与交互对齐补充契约**：
   - **`TreeView` 导航窗格契约**：
     - `TreeViewState` 提供行标签映射（`set_row_label` / `row_label` / `clear_row_labels`）、前置分区横线映射（`set_row_divider_before` / `has_row_divider_before` / `clear_row_dividers`）与单行悬停态追踪（`hovered` / `hover_at` / `clear_hover`），避免整棵树共享父容器悬停态；
     - `TreeViewDesc` 提供向后兼容的导航窗格预设（`.with_navigation_pane(bool)`、`.with_indent_guides(bool)`、`.with_selection_indicator(bool)`），开启后自动切换至 `TreeViewTokens::navigation_pane()`（透明云母底 `NAV_BG` + `NAV_SELECTED` 圆角选中态）、关闭缩进参考竖线与蓝色左侧强调条；
     - 发射阶段始终以布局求解后的最终 `desc.rect` 重算行矩形并在容器边界内施加 `push_clip` / `pop_clip`，单行标签经 `elide_text_with_measure(..., ElideMode::End)` 尾部省略，内容溢出时自动注册滚轮区域并发射右侧细滚动条。
   - **`AddressBar` Windows 11 几何与双模编辑契约**：
     - 默认深度键提升至 `z = 450`（与 `TreeViewDesc` 对齐），确保嵌入带背景填充的 `BoxSurface` 容器时不被遮挡；`ViewBuilder::address_bar` 在未显式定高（`height <= 0.0`）时默认回退至 `ADDRESS_BAR_HEIGHT (32.0)`；
     - 搜索框非折叠态采用 Windows 11 标准布局：放大镜图标左置（`sr.origin.x + 8.0`）、清除按钮右置、文本区严格落在两者之间；面包屑区在 `root_icon_rect` 与首个分段之间新增 `root_chevron_rect`（`›` 分隔符），分段宽度估算针对 CJK 宽字符做双倍宽度补偿并上限钳制至 `220px`；
     - 未悬停面包屑分段常态为透明（仅悬停绘制 `Token::HOVER`、按下绘制 `Token::PRESSED`），且仅当 `hovered_hit` 精确命中对应导航按钮时才高亮，修复 `hovered_hit == None` 误高亮 `NavBack` 的缺陷；
     - 发射阶段对路径区与搜索区分别施加 GPU `push_clip` / `pop_clip` 与尾部省略，通过 `emit_address_bar_with_visual` / `emit_address_bar_text_layer` 区分主文本与 `Token::TEXT_SECONDARY` 次级占位文本，并在编辑态与搜索聚焦态绘制选区高亮、光标竖线与 IME 预编辑下划线（保持 24 态图元数恒定）。

5. **Windows 11 中性薄纱交互、统一彩色图标与菜单/标签交互闭环契约**：
   - **Win11 中性薄纱令牌与交互状态机**：
     - `unbound-theme` 的 `ProductPreset::AssetManager` 预设将 `Token::NAV_SELECTED`、`Token::NAV_HOVER`、`Token::HOVER` 与 `Token::PRESSED` 对齐至 `win11::dark()` / `win11::light()` 的中性半透明薄纱色板（浅色模式中性灰半透、深色模式中性白半透），消除非标偏色；
     - `TreeView` 将展开箭头子热区（`tree_toggle_node`）归属到所属行，确保单行 `Hovered` / `Pressed` 状态连续无闪烁；`AddressBar` 导航按钮/面包屑分段与 `Button` 微妙变体（Subtle）统一接入逐部件独立 `PointerPhase` 状态机，同时保持 24 态图元数恒定。
   - **统一 `WidgetIcon` 双通道彩色图标与 `PrimType::Image` 原色保真不变量**：
     - `unbound-widgets::shell` 提供统一 `WidgetIcon` 枚举（`None` / `Vector(ShellIcon)` / `ExternalTexture(u64)`）与 `emit_widget_icon_primitives` 发射器，统一服务于 `TreeView`（`TreeRowIcon = WidgetIcon`）、`AddressBar`、`Button`、`TabView` 与 `ContextMenu`；
     - 发射外部彩色纹理 `PrimType::Image` 时必须固定 `fill = Token::NONE`，禁止片元着色器将彩色纹理像素与前景色令牌相乘（避免浅色模式下发黑）；禁用态统一通过 `z_and_opacity` 降低不透明度为 `WINUI_DISABLED_OPACITY`；
     - `unbound-app` 发射阶段经 `AssetHub::external_texture_slot` 统一解析外部纹理句柄，尚未驻留 GPU 槽位的句柄安全退化为 `WidgetIcon::None`，绝不泄露原始句柄号至 GPU 图元。
   - **`TabView` 载荷重定位、关闭按钮与独立子热区事件闭环**：
     - `WidgetOutput::append_child_primitives` 在合并子控件输出前自动按当前 `payloads.len()` 重定位 `extra_payload_idx`，消除标签图标、关闭叉号 `×` 与字形载荷索引偏移；
     - 冻结确定性子热区键 `tab_item_node(tabs_key, index)`、`tab_close_node(tabs_key, index)` 与 `tab_add_node(tabs_key)`，实现每个标签页主体、关闭按钮与 `+` 新建按钮独立派生 `Hovered` / `Pressed` 态而互不串扰；
     - `AppContext` 提供 `take_tab_close_request`（左键点击 `×` 与中键点击可关闭标签，且不误切激活标签）、`take_tab_add_request`（`+` 新建按钮）与 `tab_context_target` / `take_tab_context_target`（右键命中具体标签，不误切激活标签）。
   - **`ContextMenu` / `MenuFlyout` Win11 完整渲染与点击回传闭环**：
     - 冻结 Win11 几何常量：`CONTEXT_MENU_ITEM_HEIGHT = 32.0`、`MENU_PANEL_PADDING = 4.0`、`MENU_ITEM_RADIUS = 4.0`、`MENU_ICON_SIZE = 16.0`、`MENU_SEPARATOR_HEIGHT = 9.0`，同时保留既有 `MENU_ITEM_HEIGHT (37.0)`、`MenuDesc::new` 与 `emit_menu` 签名向后兼容；
     - `MenuDesc::context_menu` 无标题头，渲染 `8px` 圆角（`MENU_RADIUS`）`Token::OVERLAY_BG` 面板、`1px` `Token::CARD_BORDER` 描边 + `MENU_SHADOW` 解析阴影、四边 `4px` 内边距与 `9px` 居中 `1px` `Token::DIVIDER` 分隔线；`content_height()`、`item_rect(index)`、`item_at_offset` 与 `ContextMenuState::desc_at` 共享同一累计高度公式；
     - `emit_menu_with_rows` 实现三列对齐布局：左侧 `16×16` `WidgetIcon` 图标列（支持彩色 `ExternalTexture` 与勾选态 `ShellIcon::Check`，同级任一项含图标或勾选态则整列对齐）+ 左对齐主标签（`Token::TEXT_PRIMARY`、禁用态 `Token::TEXT_DISABLED`、危险项 `Token::ERROR`）+ 右对齐次级色快捷键（`Token::TEXT_SECONDARY`）或子菜单 `›` 箭头（`ShellIcon::ChevronRight` / `Token::SUBMENU_ARROW`）；
     - 每个可选非分隔线项在 `PORTAL_LAYER` 注册独立子热区 `menu_item_node(node, index)`，悬停填充 `Token::HOVER`（危险项叠加柔和错误底色 `Token::ERROR_SOFT`）、按压填充 `Token::PRESSED`，且固定菜单在不同交互阶段保持图元数恒定；分隔线与禁用项不注册可点击热区；
     - `unbound-app` 在 `WidgetSpec::ContextMenu` 发射阶段自动完成 `text_ctx.shape_line` + `elide_text_with_measure` 排版、`resolve_menu_item_icons` 外部纹理槽解析与逐项 `PointerPhase` 派生；左键点击可选菜单项时写入一次性激活记录，由 `AppContext::take_context_menu_activated(key)` 向宿主回传命中项索引。

## 后果

- 任何使用 `unbound-ui` 的桌面文件管理器或多标签工作台均可直接通过标准 `ViewBuilder` API 组装出与 Windows 11 资源管理器视觉及交互一致的三层壳层（融合标签标题栏 → 双模地址/搜索栏 → 紧凑 `CommandBar` + 虚拟化导航树）与原生密度上下文菜单，无需在应用层自建绕行控件。
- `WidgetIcon` 统一抽象使矢量图标与平台原生彩色纹理图标可在 `TreeView`、`AddressBar`、`Button`、`TabView` 与 `ContextMenu` 之间零破坏切换并保证原色保真。
- `TabView`、`TreeView`、`AddressBar` 与 `MenuFlyout` 的默认构造与图元数恒定门禁保持向后兼容，不影响现有批量像素差分基线。
