# 05：切片 4 —— 浮层日期类控件对齐与兜底通道彻底封死

**要构建什么：** 
打通浮层、日期与辅助类控件（`Flyout`、`MenuFlyout`、`InfoBar`、`Expander`、`SplitView`、`CalendarView`、`TimePicker`、`PersonPicture`、`FontIcon`、`AppBarButton`、`AppBarToggleButton`、`Pivot`）的离屏差分对齐；在全量 66 个基准控件独立分支就绪后，将差分器中的通用展台演示兜底通道（`other => catalog.rs`）彻底删除封死；并执行全量离屏回归测试，达成 100% 门禁闭环。

**阻塞项：** 02：切片 1 —— 布局容器与文本编辑控件对齐，03：切片 2 —— 导航与命令条类控件对齐，04：切片 3 —— 集合与树形列表类控件对齐

**状态：** ready-for-agent

- [ ] 补齐 `Flyout`、`MenuFlyout`、`InfoBar`、`Expander`、`SplitView`、`CalendarView`、`TimePicker`、`PersonPicture`、`FontIcon`、`AppBarButton`、`AppBarToggleButton`、`Pivot` 12 个控件的独立镜像分支
- [ ] **收缩（Contract）**：将 `parity.rs` 中的 `other => catalog.rs` 兜底彻底移除，替换为显式未实现错误（`Err(format!("unimplemented parity control: {other}"))`），杜绝隐式回退
- [ ] 运行单 Vulkan 设备复用的批量差分器对全量 66 个基准控件执行离屏差分
- [ ] 产出最终 `target/parity/report.json`，验证全量用例门禁全部通过（纯光栅化抗锯齿残差经 ADR 0010 白名单核验）
