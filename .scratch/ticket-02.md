## Parent

Part of #74

## What to build

在多控件流式装箱的基础上，当全量待采控件状态超出 1920×1080 DIP 视口安全上限时，自动进行视口溢出安全分页；在同一个 WinUI 3 Window 实例内实现 `Host.Children` 就地清空与装载下一页的连续翻页状态机，并支持 `--save-sheet` 导出 `page_{index}_sheet.png` 及坐标索引 JSON。

消除跨批次重新创建与销毁 HWND 导致的窗口闪烁与 DWM 重建开销，实现全量 66 个基准控件单主题在 2~3 页内平稳连续翻页采集。

## Acceptance criteria

- [ ] 装箱规划器在槽位行高度累加超过 1080 DIP 安全上限时，自动截断当前页并顺延开启下一页
- [ ] 整个批处理会话始终复用同一个 `Window` 实例，翻页时在主线程安全清空 `Host.Children` 并挂载新页控件树，无窗口重建与屏幕闪烁
- [ ] 每一页独立等待 3 帧渲染与 350ms 动效收敛，单页导出整图并切分单图后自动触发翻页，直至所有分页全部处理完毕后平稳退出应用
- [ ] 传入 `--save-sheet` 参数时，除单图切片外，按页码输出 `page_{pageIndex:D2}_{theme}_{backdrop}_sheet.png`，并同步写入记录本页控件列表与各槽位相对包围盒的同名 `.json` 文件
- [ ] 全量 66 个基准控件在 `--batch --no-cache` 下稳定切分为 2~3 页，总采集时间缩短至 5 秒内且零崩溃

## Blocked by

- #75
