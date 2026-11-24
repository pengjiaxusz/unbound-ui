# ADR 0034：基于 4 KiB LUT 逐帧插值的框架级主题过渡与后景协同契约

- 状态：已采纳（2026-11-24 修订：结合 Win11 实机验收结果，将原生 DWM 翻转时机由中点改为起点 `t = 0` 同步起跑，并收口统一主题过渡驱动与全框架不变量）
- 日期：2026-11-24
- 关联：延续 `unbound-theme` 零重排/零实例重传不变量；衔接 ADR 0014（窗口外观账本）、ADR 0023（多窗口共享设备）、ADR 0030（可见性驱动节流）与 ADR 0031（控件能力边界）

## 背景

当前框架在深浅色主题（`Light` / `Dark`）、产品预设（`ProductPreset`）或品牌强调色（`AccentBrand`）切换时，直接将目标调色板一次性写入 4 KiB 主题 LUT，界面在单帧内从纯黑跳变至纯白（或反之），暗光环境下对人眼造成明显闪屏刺激。

`unbound-ui` 的渲染管线在结构上已将图元实例与具体颜色解耦：`uber.frag` 与 `common.glsl` 中全部图元的填充、描边、阴影与线性渐变均经由 `theme_lut_bda` 实时查表，硬件清屏色亦由 `Token::SURFACE` 同步派生，全部标准控件不存在按 `ThemeKind` 分支换令牌或绕过 LUT 硬编码前景色的路径。但要让**整个框架与所有宿主**无遗漏地享有平滑过渡，必须同时解决五类跨模块边界约束：

1. **多帧在途数据竞争**：`Renderer::lut_buffer` 原为单份 4 KiB 宿主可见缓冲；在 `frames_in_flight >= 2` 下连续逐帧写入会与 GPU 尚未退役的上一帧产生读写竞争。
2. **色彩感知与透明端点伪影**：在线性 RGB 空间做黑↔白插值会导致暗部陡跳、亮部发平；若某令牌在单侧主题为全透明（`alpha == 0`），直接插值 RGB 会引入暗边或灰边。
3. **双路径后景的真实物理时序**：Win11 实机验收证实，原生路径收到 `DWMWA_USE_IMMERSIVE_DARK_MODE` 后**DWM 自身即会播放约 200–300ms 的系统渐变**，并非瞬切；自绘路径的壁纸空间纹理则在后台线程按代次异步重建，到达时间可能滞后于前景过渡起点。
4. **宿主双轨分叉风险**：若过渡状态机仅写在 `WindowedRunner` / `MultiWindowCoordinator` 内部，自建主循环的宿主（如 `apps/showcase`）会绕过框架过渡直接裸写 LUT，导致前景瞬切而底层 Mica 缓慢渐变，形成“半黑半白”严重撕裂。
5. **无障碍、多窗口与差分测试门禁**：开启系统「减弱动画」的用户不应被迫观看过渡动画；不可见（`Suspended`）窗口不应被过渡强行唤醒；无头、屏幕捕获对照与拼版差分采集必须保持确定性瞬切，避免采到中间帧并污染用例指纹。

## 决策

- **过渡形态与默认曲线**：框架默认采用**基于 4 KiB LUT 的全量颜色渐变**，每帧在 CPU 插值 512 个令牌并写入当帧 LUT 槽位，**严格维持“切主题不触发布局重算、不重传图元实例、不新增离屏渲染 Pass”的不变量**。默认时长 **240ms**，采用 `CubicBezier` ease-out 曲线；提供应用级配置入口允许覆写时长（设为 `0` 即关闭过渡）。
- **GPU LUT 环形缓冲**：将 `Renderer::lut_buffer` 扩展为按 `frames_in_flight` 分槽的环形缓冲（每槽 4 KiB，总计 `frames_in_flight × 4 KiB`），每帧延后至目标槽位围栏退役后再刷写并通过 push constant 的 `theme_lut_bda` 指向该槽位偏移，彻底消除连续逐帧更新的 CPU/GPU 数据竞争；过渡收敛后调度器照常回落至 0% 占用。
- **统一主题过渡驱动（`ThemeTransitionDriver`）**：在 `unbound-app` 提供唯一的框架级主题过渡驱动器，统一封装 `ThemeTransition` 状态机、单调时钟、首帧瞬切、系统减弱动画查询与 LUT 上传；**所有交互式宿主（`WindowedRunner`、`MultiWindowCoordinator`、`apps/showcase` 及自定义主循环宿主）必须统一消费此驱动**，并在 `is_active()` 或自绘后景 `needs_frame()` 为真时持续维持帧唤醒，直至收敛归零。
- **Oklab 感知插值与透明端点处理**：
  - RGB 分量采用 `unbound-math::Color::lerp_oklab` 做感知均匀插值，Alpha 分量保持线性插值；
  - `Token::NONE`（槽位 0）恒定保持全透明，不参与插值；
  - 当任一端点的 `alpha == 0` 时，插值期间沿用非透明端的 RGB 分量、仅插值 Alpha，消除黑边与灰边伪影。
- **中途重入与平滑折返**：过渡尚未结束时若再次触发主题、预设或强调色变更，将当前帧正在呈现的插值调色板冻结为新的起点 `from`，目标切换为新 `to`；若为反向折返（回到先前方向），按已走进度按比例缩短时长（`duration * progress`），避免顿挫与跳变。
- **双路径后景协同（修订）**：
  - **原生路径**：在**过渡起点（`t = 0`）即刻经由 `WindowAppearance` 账本下发 `DWMWA_USE_IMMERSIVE_DARK_MODE`**（并继续由账本按已下发值去重），使 Windows DWM 原生 Mica 的系统渐变与框架前景 240ms Oklab LUT 渐变同一起跑线同步完成，消除前后错拍。
  - **自绘路径**：前景立即启动 LUT 过渡而不阻塞等待后台线程；新代次壁纸空间纹理就绪后，保留上一代纹理并在底层以双图元透明度叠化淡入（取前景剩余过渡时长，保底不低于 `150ms`），叠化完成后释放旧纹理。
- **多窗口与可见性节流协同**：多窗口应用发生全局主题切换时，所有可见窗口共享同一触发时间戳同步推进；遵循 ADR 0030，处于 `Suspended` 停绘态的窗口不因主题过渡被唤醒，待下次恢复可见时直接吸附至目标终态（`t = 1.0`）。
- **触发范围与强制瞬切逃生舱**：
  - **走过渡**：运行期深浅色切换、跟随系统 `ThemeChanged`、以及 `ProductPreset` / `AccentBrand` 切换。
  - **强制瞬切（`duration = 0`）**：窗口首帧初始化、操作系统开启「减弱动画」（在 `unbound-platform` 补齐 Win32 `SPI_GETCLIENTAREAANIMATION` 与 Linux 桌面偏好查询及默认开启兜底）、应用显式配置时长为 `0`、以及无头渲染、屏幕捕获对照与差分/拼版采集模式。
- **全框架防回归不变量**：
  1. **控件与页面语义令牌单一性**：所有控件与业务页面发射图元时必须使用跨主题稳定的语义 `Token`，禁止按 `is_dark()` / `ThemeKind` 分支切换不同令牌或硬编码前景色值。
  2. **宿主统一驱动与帧唤醒契约**：所有交互式宿主必须经由 `ThemeTransitionDriver` 上传 LUT，且在主题过渡或自绘后景叠化活跃期间持续驱动重绘，禁止裸写 `Renderer::write_theme_lut` 绕过过渡（离屏/差分/测试显式走瞬切入口）。

## 备选方案

- **将原生 DWM 翻转延迟至中点（`t >= 0.5`，初版方案，已废止）**：初版假设 DWM 翻转是瞬时的；Win11 实机截图证实 DWM 原生 Mica 本身即执行系统级平滑渐变，延迟 120ms 反而加剧前景与背景错拍，故修订为 `t = 0` 立即同步下发。
- **将时间插值直接下沉至 `unbound-gpu::Renderer` 内部**：虽能覆盖裸用渲染器的调用方，但会让底层纯图形执行器反向耦合时钟、无障碍偏好与多窗同步时钟，并破坏离屏单帧测试的确定性；在 `unbound-app` 提供唯一 `ThemeTransitionDriver` 并收口全仓宿主更符合分层边界。
- **离屏快照空间过渡（如点击点圆形扩散或全屏叠化）**：需分配瞬态全屏离屏纹理并新增合成 Pass，侵入帧图且抬高显存峰值，暂不作为框架默认实现。

## 影响

- `unbound-theme` 维护纯值 `ThemeTransition` 状态机，其 `effective_backdrop_theme()` 在 `t = 0` 即返回目标主题以与原生 DWM 同步起跑；
- `unbound-gpu` 以按 `frames_in_flight` 分槽的环形 LUT 缓冲承载连续逐帧写入；
- `unbound-platform` 提供跨平台 `prefers_reduced_motion()` 系统偏好查询；
- `unbound-app` 导出统一的 `ThemeTransitionDriver`，由 `WindowedRunner`、`MultiWindowCoordinator` 与 `apps/showcase` 共同消费，确保全框架所有控件与宿主零遗漏覆盖主题过渡。
