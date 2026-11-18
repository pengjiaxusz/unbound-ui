# 04：接入并校准第一批已实现基准控件（基础输入与文本类 12 个）

**要构建什么：** 将已实现的基础输入与文本类基准控件（`ToggleButton`、`SplitButton`、`RadioButton`、`RatingControl`、`ColorPicker`、`NumberBox`、`AutoSuggestBox`、`RichTextBlock`、`ProgressBar`、`ProgressRing`、`InfoBadge`、`ToolTip`）纳入参考截图宿主拼版目录与本框架批量差分器，并修复所有未通过 ROI 四重门禁的外观与固有尺寸偏差。

**阻塞项：** 02：确立三合一控件清单并以扩展模式接入 WinUI 3 规范名映射, 03：校准首批 6 个试点基准控件通过有效对比区（ROI）四重门禁

**状态：** ready-for-agent

- [ ] 参考截图宿主与本框架差分器均支持上述 12 个控件的状态拼版采集与离屏渲染
- [ ] 输入类控件（`NumberBox`、`AutoSuggestBox`）严格对齐 32px 标准高度、4px 圆角与聚焦态底部 2px 强调色描边
- [ ] 上述 12 个控件在浅色与深色模式下的所有支持状态全部通过 ROI 四重门禁，且保持 24 态图元数量恒定
