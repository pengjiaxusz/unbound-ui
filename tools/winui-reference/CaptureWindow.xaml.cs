using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace WinUiReference;

/// <summary>
/// 截图窗口：在固定的确定性 1920×1080 DIP 瞬态画布上进行多控件 2D 流式装箱，
/// 共享单次 3 帧渲染与 350ms 动效收敛等待，单次导出大图并按槽位相对几何边界切出紧凑单图与 JSON。
/// </summary>
public sealed partial class CaptureWindow : Window
{
    public const double CanvasWidthDip = 1920.0;
    public const double CanvasHeightDip = 1080.0;

    private readonly CaptureOptions _options;
    private IReadOnlyList<DynamicParityPage> _sheets = Array.Empty<DynamicParityPage>();
    private readonly Dictionary<string, (double Width, double Height)> _slotSizeCache = new();
    private readonly System.Diagnostics.Stopwatch _settle = new();
    private int _sheetIndex;
    private readonly List<(CaptureCase Case, Border SlotHost, FrameworkElement Control)> _activeSlots = new();
    private DispatcherQueueTimer? _timeoutTimer;
    private DispatcherQueueTimer? _resizeWatchdog;
    private int _renderingTicks;
    private int _captureStarted;
    private bool _settleStarted;
    private bool _loaded;
    private int _skippedCount;
    private int _capturedCount;
    private int _failedCount;
    private double _targetWidthDip = CanvasWidthDip;
    private double _targetHeightDip = CanvasHeightDip;

    public CaptureWindow(CaptureOptions options)
    {
        _options = options;
        InitializeComponent();

        Title = options.Review
            ? "WinUI Reference - review"
            : $"WinUI Reference - {options.ControlName}";

        Host.Loaded += OnHostLoaded;

        if (options.Review)
        {
            InitializeReviewUi();
        }
    }

    private DynamicParityPage ActiveSheet => _sheets[_sheetIndex];

    private void OnHostLoaded(object sender, RoutedEventArgs e)
    {
        if (_loaded)
        {
            return;
        }

        _loaded = true;
        var candidateBatches = _options.BuildCandidateBatches();
        var filteredBatches = FilterCandidateBatches(candidateBatches);
        _sheets = filteredBatches.Count > 0
            ? PlanPages(filteredBatches)
            : Array.Empty<DynamicParityPage>();

        if (_options.Review)
        {
            // 检视模式不采图、不退出：把同一套装箱规划结果直接摆进窗口，等用户翻页。
            ReviewBar.Visibility = Visibility.Visible;
            EnterReview(0);
            return;
        }

        AdvanceToNextUncachedOrStart(initial: true);
    }

    /// <summary>用例指纹比对上下文：缓存运行环境元数据与输出目录路径，减少跨方法零散传参。</summary>
    private sealed record FingerprintContext(
        string OutputDirectory,
        double Scale,
        string SdkVersion,
        string OsVersion);

    private FingerprintContext CreateFingerprintContext() =>
        new(
            Path.GetFullPath(_options.OutputDirectory),
            Host.XamlRoot?.RasterizationScale ?? 1.0,
            typeof(Application).Assembly.GetName().Version?.ToString() ?? "unknown",
            Environment.OSVersion.Version.ToString());

    /// <summary>
    /// 装箱前用例指纹增量过滤：
    /// 基于 SDK 版本、OS 版本、光栅化缩放及控件规格指纹，比对磁盘上已有的 PNG 与 JSON 切片产物。
    /// 仅保留存在至少一个未命中状态的候选控件（或在指定 --no-cache / 显式指定 --control 时保留全部）。
    /// 已全部命中缓存的控件完全不进入装箱规划器，不分配版面槽位，不参与可视树测量。
    /// 若某主题下的所有控件均已命中缓存，直接打印 [cache-hit] 并跳过当前主题所有拼版页的渲染与等待。
    /// </summary>
    private List<CandidateBatch> FilterCandidateBatches(IReadOnlyList<CandidateBatch> batches)
    {
        // 检视模式永远装载全量页面：人工评审要看的是完整矩阵，
        // 而不是「本次运行恰好过期的那几页」。
        if (_options.Refresh || _options.ExplicitControl || _options.Review)
        {
            return batches.ToList();
        }

        var context = CreateFingerprintContext();
        var filteredBatches = new List<CandidateBatch>();

        foreach (var batch in batches)
        {
            var groups = batch.Cases
                .GroupBy(c => Catalog.CanonicalName(c.ControlName), StringComparer.OrdinalIgnoreCase)
                .ToList();

            var activeCases = new List<CaptureCase>();
            var cachedControls = new List<string>();
            var uncachedControls = new List<string>();

            foreach (var group in groups)
            {
                var controlName = group.Key;
                var allCached = true;

                foreach (var c in group)
                {
                    if (!IsCaseCached(c, context))
                    {
                        allCached = false;
                        break;
                    }
                }

                if (allCached)
                {
                    _skippedCount += group.Count();
                    cachedControls.Add(controlName);
                }
                else
                {
                    activeCases.AddRange(group);
                    uncachedControls.Add(controlName);
                }
            }

            if (activeCases.Count == 0)
            {
                Console.Out.WriteLine(
                    $"[cache-hit] All cases cached for theme {batch.Theme} ({batch.Backdrop}): {batch.Cases.Count} states skipped ({string.Join(", ", cachedControls)})");
            }
            else
            {
                if (cachedControls.Count > 0)
                {
                    Console.Out.WriteLine(
                        $"[cache-hit] Theme {batch.Theme} ({batch.Backdrop}): {cachedControls.Count} controls skipped ({string.Join(", ", cachedControls)}), {uncachedControls.Count} controls pending ({string.Join(", ", uncachedControls)})");
                }

                filteredBatches.Add(new CandidateBatch(batch.Theme, batch.Backdrop, activeCases));
            }
        }

        return filteredBatches;
    }

    /// <summary>检查单个用例是否已在磁盘上存在指纹完全一致的有效 PNG 与 JSON 切片产物。</summary>
    private static bool IsCaseCached(CaptureCase c, FingerprintContext context)
    {
        var baseName = c.FileBaseName();
        var pngPath = Path.Combine(context.OutputDirectory, $"{baseName}.png");
        var jsonPath = Path.Combine(context.OutputDirectory, $"{baseName}.json");
        if (!File.Exists(pngPath) || !File.Exists(jsonPath))
        {
            return false;
        }

        try
        {
            var jsonText = File.ReadAllText(jsonPath);
            var expectedFp = Catalog.ComputeFingerprint(
                c.ControlName,
                c.State,
                c.Theme,
                c.Backdrop,
                context.Scale,
                context.SdkVersion,
                context.OsVersion);
            return jsonText.Contains($"\"fingerprint\": \"{expectedFp}\"", StringComparison.Ordinal);
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// 自适应测量单个用例的槽位期望尺寸（DesiredSize + 2 * BleedMargin），并缓存结果。
    /// 测量时临时挂载入已加载的 Host 可视树，确保 SwipeControl 等模板安全初始化。
    /// </summary>
    private (double Width, double Height) MeasureSlotSize(CaptureCase captureCase)
    {
        var key = $"{Catalog.CanonicalName(captureCase.ControlName)}_{captureCase.State.ToLowerInvariant()}";
        if (_slotSizeCache.TryGetValue(key, out var size))
        {
            return size;
        }

        if (!Catalog.TryCreate(captureCase.ControlName, out var control))
        {
            return (Catalog.SlotWidthDip, Catalog.SlotHeightDip);
        }

        Catalog.TryApplyState(control, captureCase.State, out _);
        control.RequestedTheme = captureCase.Theme == "dark" ? ElementTheme.Dark : ElementTheme.Light;

        var slotHost = new Border
        {
            Width = Catalog.SlotWidthDip,
            Height = Catalog.SlotHeightDip,
            Child = control,
        };
        Host.Children.Add(slotHost);

        control.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
        var desired = control.DesiredSize;
        var widthDip = Math.Max(
            Catalog.SlotWidthDip,
            Math.Ceiling(desired.Width) + 2.0 * Catalog.BleedMarginDip);
        var heightDip = Math.Max(
            Catalog.SlotHeightDip,
            Math.Ceiling(desired.Height) + 2.0 * Catalog.BleedMarginDip);

        Host.Children.Remove(slotHost);

        size = (widthDip, heightDip);
        _slotSizeCache[key] = size;
        return size;
    }

    /// <summary>
    /// 视口溢出安全分页与装箱规划器：
    /// 在固定的确定性 1920×1080 DIP 画布内按槽位高度降序（FFDH）流式装箱摆放各用例槽位；
    /// 当新行高度累加超过 1080 DIP 安全上限时，自动截断当前页并顺延开启下一页（pageIndex++）。
    /// </summary>
    private List<DynamicParityPage> PlanPages(IReadOnlyList<CandidateBatch> batches)
    {
        var pages = new List<DynamicParityPage>();

        foreach (var batch in batches)
        {
            var pageIndex = 0;
            var currentPageSlots = new List<PlannedSlot>();
            double currentX = 0.0;
            double currentY = 0.0;
            double currentRowHeight = 0.0;

            // 按槽位高度降序装箱（FFDH：First-Fit Decreasing Height），消除同行高矮混排造成的纵向空隙，
            // 确保全量 66 个基准控件单主题稳定收敛在 2~3 页内。
            var sortedCases = batch.Cases
                .OrderByDescending(c => MeasureSlotSize(c).Height)
                .ThenBy(c => Catalog.CanonicalName(c.ControlName), StringComparer.Ordinal)
                .ThenBy(c => c.State, StringComparer.Ordinal)
                .ToList();

            for (var i = 0; i < sortedCases.Count; i++)
            {
                var c = sortedCases[i];
                var (w, h) = MeasureSlotSize(c);

                // 步骤 1：判断当前行在 X 轴上是否放得下当前槽位
                if (currentX + w > CanvasWidthDip && currentX > 0.0)
                {
                    // 需要换行：计算换行后新行的 Y 起点与初始行高
                    var nextY = currentY + currentRowHeight;
                    var nextRowHeight = h;

                    // 步骤 2：视口溢出安全分页判断：
                    // 当添加新行会导致 nextY + nextRowHeight > 1080 DIP 安全上限时，
                    // 自动截断当前页并顺延开启下一页（pageIndex++）
                    if (nextY + nextRowHeight > CanvasHeightDip)
                    {
                        pages.Add(new DynamicParityPage(batch.Theme, batch.Backdrop, currentPageSlots, pageIndex++));
                        currentPageSlots = new List<PlannedSlot>();
                        currentX = 0.0;
                        currentY = 0.0;
                        currentRowHeight = 0.0;
                    }
                    else
                    {
                        currentY = nextY;
                        currentX = 0.0;
                        currentRowHeight = 0.0;
                    }
                }

                // 步骤 3：极端单槽位高度防御：若单槽位高度超出了视口剩余高度且当前页已有控件，截断开启新页
                if (currentY + h > CanvasHeightDip && currentPageSlots.Count > 0)
                {
                    pages.Add(new DynamicParityPage(batch.Theme, batch.Backdrop, currentPageSlots, pageIndex++));
                    currentPageSlots = new List<PlannedSlot>();
                    currentX = 0.0;
                    currentY = 0.0;
                    currentRowHeight = 0.0;
                }

                currentPageSlots.Add(new PlannedSlot(c, currentX, currentY, w, h));
                currentX += w;
                currentRowHeight = Math.Max(currentRowHeight, h);
            }

            if (currentPageSlots.Count > 0)
            {
                pages.Add(new DynamicParityPage(batch.Theme, batch.Backdrop, currentPageSlots, pageIndex++));
            }
        }

        // 确保首轮测量留下的临时子节点被彻底清空
        Host.Children.Clear();
        return pages;
    }

    /// <summary>
    /// 跳过所有已命中缓存的拼版批次；若存在未命中的用例则装载该拼版并启动渲染等待循环。
    /// </summary>
    private void AdvanceToNextUncachedOrStart(bool initial)
    {
        if (!initial)
        {
            _sheetIndex++;
        }

        var context = CreateFingerprintContext();
        while (_sheetIndex < _sheets.Count)
        {
            var sheet = _sheets[_sheetIndex];
            if (!_options.Refresh && !_options.ExplicitControl && AllCasesCached(sheet, context))
            {
                _skippedCount += sheet.Slots.Count;
                Console.Out.WriteLine(
                    $"[cache-hit] Skipping Page {sheet.PageIndex} ({sheet.Theme}, {sheet.Backdrop}) with {sheet.Slots.Count} states ({string.Join(", ", sheet.ControlNames)})");
                _sheetIndex++;
                continue;
            }

            if (!LoadSheet(sheet))
            {
                Environment.ExitCode = 2;
                Application.Current.Exit();
                return;
            }

            ResizeClientAreaForSheet();
            StartCaptureLoop();
            return;
        }

        Console.Out.WriteLine(
            $"Capture complete: added {_capturedCount} cases, failed {_failedCount} cases, cache hit {_skippedCount} cases.");
        Application.Current.Exit();
    }

    /// <summary>检查当前拼版中的所有槽位是否均在磁盘上拥有指纹匹配的 PNG 与 JSON。</summary>
    private static bool AllCasesCached(DynamicParityPage sheet, FingerprintContext context)
    {
        foreach (var slot in sheet.Slots)
        {
            if (!IsCaseCached(slot.Case, context))
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>
    /// 把一个动态拼版页下的所有待测状态装载入 Host 画布，直接应用规划好的物理坐标与尺寸。
    /// </summary>
    private bool LoadSheet(DynamicParityPage sheet)
    {
        Host.RequestedTheme = sheet.Theme == "dark" ? ElementTheme.Dark : ElementTheme.Light;
        Host.Background = Catalog.CreateBackdropBrush(sheet.Theme, sheet.Backdrop);
        Host.Children.Clear();
        _activeSlots.Clear();

        // 步骤 1：直接使用装箱规划器已计算好的槽位尺寸与绝对坐标，实例化控件并挂入 Host 画布
        foreach (var slot in sheet.Slots)
        {
            var captureCase = slot.Case;
            if (!Catalog.TryCreate(captureCase.ControlName, out var control))
            {
                Fatal.Report($"Unknown control: {captureCase.ControlName}{Environment.NewLine}Available controls: {string.Join(", ", Catalog.ControlNames)}");
                return false;
            }

            if (!Catalog.TryApplyState(control, captureCase.State, out var stateError))
            {
                Fatal.Report(stateError);
                return false;
            }

            control.RequestedTheme = Host.RequestedTheme;
            // ProgressRing 的弧线颜色不走可覆盖的资源键，而是直接取系统强调色；
            // 这里显式指定前景色，把基线钉死为 Windows 11 默认蓝/青。
            if (control is ProgressRing ring)
            {
                var accent = control.RequestedTheme == ElementTheme.Dark
                    ? Windows.UI.Color.FromArgb(255, 0x60, 0xCD, 0xFF)
                    : Windows.UI.Color.FromArgb(255, 0x00, 0x5F, 0xB8);
                ring.Foreground = new SolidColorBrush(accent);
            }


            var slotHost = new Border
            {
                Width = slot.Width,
                Height = slot.Height,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                Background = Catalog.CreateBackdropBrush(sheet.Theme, sheet.Backdrop),
                Child = control,
            };

            Canvas.SetLeft(slotHost, slot.X);
            Canvas.SetTop(slotHost, slot.Y);
            Host.Children.Add(slotHost);
            _activeSlots.Add((captureCase, slotHost, control));
        }

        // 步骤 2：在可视树内触发布局测量，初始化模板（如 SwipeControl）
        for (var idx = 0; idx < _activeSlots.Count; idx++)
        {
            var (_, _, control) = _activeSlots[idx];
            control.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
        }

        Host.UpdateLayout();
        _renderingTicks = 0;
        _captureStarted = 0;
        _settleStarted = false;
        return true;
    }

    /// <summary>
    /// 等待窗口 resize 生效后才开始 350ms 动画收敛计时，避免在旧尺寸上等待。
    /// </summary>
    private void StartCaptureLoop()
    {
        if (SizeMatchesTarget())
        {
            BeginSettle();
            return;
        }

        Host.SizeChanged += OnHostSizeChangedForCapture;
        // 兜底：窗口管理器未回报尺寸变化时，3 秒后仍按当前布局继续采集。
        _resizeWatchdog = DispatcherQueue.CreateTimer();
        _resizeWatchdog.Interval = TimeSpan.FromSeconds(3);
        _resizeWatchdog.IsRepeating = false;
        _resizeWatchdog.Tick += (_, _) => BeginSettle();
        _resizeWatchdog.Start();
    }

    private void OnHostSizeChangedForCapture(object sender, SizeChangedEventArgs e)
    {
        if (SizeMatchesTarget())
        {
            BeginSettle();
        }
    }

    private bool SizeMatchesTarget() =>
        Math.Abs(Host.ActualWidth - _targetWidthDip) <= 1.5 &&
        Math.Abs(Host.ActualHeight - _targetHeightDip) <= 1.5;

    private void BeginSettle()
    {
        if (_settleStarted)
        {
            return;
        }

        _settleStarted = true;
        Host.SizeChanged -= OnHostSizeChangedForCapture;
        _resizeWatchdog?.Stop();
        _resizeWatchdog = null;

        _settle.Restart();
        _renderingTicks = 0;
        CompositionTarget.Rendering += OnCompositionRendering;

        // 兜底：远程桌面/最小化等场景可能收不到渲染回调，超时后仍然尝试截图。
        _timeoutTimer = DispatcherQueue.CreateTimer();
        _timeoutTimer.Interval = TimeSpan.FromSeconds(10);
        _timeoutTimer.IsRepeating = false;
        _timeoutTimer.Tick += (_, _) => BeginCapture("Rendering event timeout (10s), capturing directly");
        _timeoutTimer.Start();
    }

    private void OnCompositionRendering(object? sender, object e)
    {
        _renderingTicks++;

        // 焦点必须在控件进入可视树并获得一个渲染帧之后申请。
        if (_renderingTicks == 1)
        {
            foreach (var (captureCase, _, control) in _activeSlots)
            {
                Catalog.ApplyFocus(control, captureCase.State);
            }
        }

        // ProgressRing 的不确定动画由 AnimatedVisualPlayer 驱动，属于合成动画，
        // 不受 Timeline.AllowDependentAnimations 影响。前两个渲染帧内停止播放器，
        // 让所有采集都落在同一初始帧上。
        if (_renderingTicks <= 2)
        {
            foreach (var (_, _, control) in _activeSlots)
            {
                FreezeAnimations(control);
            }
        }

        // 同一拼版内的所有状态共享这一次 350ms VisualState 动画收敛等待。
        if (_renderingTicks < 3 || _settle.ElapsedMilliseconds < 350)
        {
            return;
        }

        BeginCapture(warning: null);
    }

    /// <summary>ProgressRing 冻结帧的归一化播放进度（取动画中段的稳定弧段）。</summary>
    private static readonly double FrozenRingProgress =
        double.TryParse(
            Environment.GetEnvironmentVariable("PROGRESS_RING_FRAME"),
            NumberStyles.Float,
            CultureInfo.InvariantCulture,
            out var progress)
            ? progress
            : 0.4;

    /// <summary>
    /// 递归停止模板内的合成动画播放器（ProgressRing 的不确定动画），并固定到指定进度。
    /// </summary>
    private static void FreezeAnimations(DependencyObject root)
    {
        var count = VisualTreeHelper.GetChildrenCount(root);
        for (var index = 0; index < count; index++)
        {
            var child = VisualTreeHelper.GetChild(root, index);
            if (child is AnimatedVisualPlayer player)
            {
                player.Stop();
                player.SetProgress(FrozenRingProgress);
            }
            FreezeAnimations(child);
        }
    }

    private void BeginCapture(string? warning)
    {
        if (Interlocked.Exchange(ref _captureStarted, 1) != 0)
        {
            return;
        }

        CompositionTarget.Rendering -= OnCompositionRendering;
        _timeoutTimer?.Stop();

        if (warning is not null)
        {
            Fatal.Report(warning);
        }

        _ = CaptureSheetAsync(advance: true);
    }

    /// <summary>
    /// 把当前拼版导出为切片与切片 JSON。
    ///
    /// `advance` 为真时继续推进到下一页（自动批采）；检视模式的「截图本页」传假，
    /// 导出后停在当前页，窗口保持打开。
    /// `forceSheet` 强制落盘拼版总览图，用于人工留证。
    /// </summary>
    private async Task CaptureSheetAsync(bool advance, bool forceSheet = false)
    {
        const int maxAttempts = 3;

        try
        {
            var outputDirectory = Path.GetFullPath(_options.OutputDirectory);
            Directory.CreateDirectory(outputDirectory);

            byte[]? sheetPixels = null;
            var sheetWidth = 0;
            var sheetHeight = 0;
            var scale = 1.0;
            List<SlotPixelRegion>? regions = null;
            bool[]? valid = null;

            // 截图后必须在 controlBounds 内检测到非背景像素；失败重试最多 3 次，
            // 仍失败则整例失败并跳过落盘，避免把空白图写入参考集。
            for (var attempt = 1; attempt <= maxAttempts; attempt++)
            {
                var bitmap = new RenderTargetBitmap();
                await bitmap.RenderAsync(Host);

                var pixelBuffer = await bitmap.GetPixelsAsync();
                var pixels = new byte[checked((int)pixelBuffer.Length)];
                using (var pixelReader = DataReader.FromBuffer(pixelBuffer))
                {
                    pixelReader.ReadBytes(pixels);
                }

                sheetPixels = pixels;
                sheetWidth = bitmap.PixelWidth;
                sheetHeight = bitmap.PixelHeight;
                scale = Host.XamlRoot?.RasterizationScale ?? 1.0;

                regions = BuildSlotRegions(scale, sheetWidth, sheetHeight);
                valid = new bool[regions.Count];
                var allValid = true;
                for (var index = 0; index < regions.Count; index++)
                {
                    var (captureCase, _, _) = _activeSlots[index];
                    valid[index] = HasNonBackgroundPixels(sheetPixels, sheetWidth, sheetHeight, regions[index]);
                    if (!valid[index])
                    {
                        allValid = false;
                    }
                }

                if (allValid)
                {
                    break;
                }

                if (attempt < maxAttempts)
                {
                    await Task.Delay(150);
                }
            }

            if (sheetPixels is null || regions is null || valid is null)
            {
                throw new InvalidOperationException("RenderTargetBitmap returned no pixels");
            }

            // 可选保留拼版总览图与槽位坐标索引供人工速览与全局核验
            if (_options.KeepSheets || forceSheet)
            {
                var sheetName = $"page_{ActiveSheet.PageIndex:D2}_{ActiveSheet.Theme}_{ActiveSheet.Backdrop}_sheet.png";
                var sheetPath = Path.Combine(outputDirectory, sheetName);
                await WriteBgraPngAsync(sheetPath, sheetWidth, sheetHeight, sheetPixels);

                var jsonName = $"page_{ActiveSheet.PageIndex:D2}_{ActiveSheet.Theme}_{ActiveSheet.Backdrop}_sheet.json";
                var jsonPath = Path.Combine(outputDirectory, jsonName);
                var sheetMeta = new
                {
                    pageIndex = ActiveSheet.PageIndex,
                    theme = ActiveSheet.Theme,
                    backdrop = ActiveSheet.Backdrop,
                    sheetWidth,
                    sheetHeight,
                    rasterizationScale = scale,
                    slots = _activeSlots.Select((slot, idx) => new
                    {
                        control = Catalog.CanonicalName(slot.Case.ControlName),
                        state = slot.Case.State.ToLowerInvariant(),
                        cropBounds = new
                        {
                            x = regions[idx].CropX,
                            y = regions[idx].CropY,
                            width = regions[idx].CropWidth,
                            height = regions[idx].CropHeight,
                        },
                        controlBounds = new
                        {
                            x = Math.Round(regions[idx].LocalBoundsDip.X, 2),
                            y = Math.Round(regions[idx].LocalBoundsDip.Y, 2),
                            width = Math.Round(regions[idx].LocalBoundsDip.Width, 2),
                            height = Math.Round(regions[idx].LocalBoundsDip.Height, 2),
                        },
                    }),
                };
                var json = JsonSerializer.Serialize(sheetMeta, new JsonSerializerOptions { WriteIndented = true });
                await File.WriteAllTextAsync(jsonPath, json);
                Console.Out.WriteLine($"Wrote contact sheet: {sheetPath} ({sheetWidth}x{sheetHeight})");
            }

            for (var index = 0; index < _activeSlots.Count; index++)
            {
                var (captureCase, _, control) = _activeSlots[index];
                var baseName = captureCase.FileBaseName();

                if (!valid[index])
                {
                    _failedCount++;
                    Environment.ExitCode = 1;
                    Console.Error.WriteLine(
                        $"[fail] {baseName}: no non-background pixels within controlBounds after {maxAttempts} attempts; skipped");
                    continue;
                }

                var region = regions[index];
                var croppedBytes = CropBgraRegion(
                    sheetPixels,
                    sheetWidth,
                    sheetHeight,
                    region.CropX,
                    region.CropY,
                    region.CropWidth,
                    region.CropHeight);

                var pngPath = Path.Combine(outputDirectory, $"{baseName}.png");
                await WriteBgraPngAsync(pngPath, region.CropWidth, region.CropHeight, croppedBytes);

                await WriteMetaAsync(
                    outputDirectory,
                    baseName,
                    captureCase,
                    region.CropWidth,
                    region.CropHeight,
                    region.LocalBoundsDip,
                    control);

                _capturedCount++;
                Console.Out.WriteLine($"Wrote {pngPath} ({region.CropWidth}x{region.CropHeight})");
            }
        }
        catch (Exception ex)
        {
            Fatal.Report($"Capture failed: {ex}");
            Environment.ExitCode = 1;
            // 检视模式是长驻窗口：单页导出失败只报告，不把整个应用带走。
            if (!_options.Review)
            {
                Application.Current.Exit();
            }
            return;
        }

        if (advance)
        {
            AdvanceToNextUncachedOrStart(initial: false);
        }
    }

    /// <summary>单个槽位在拼版像素坐标系中的裁剪框、控件区域与背景采样点。</summary>
    private readonly record struct SlotPixelRegion(
        int CropX,
        int CropY,
        int CropWidth,
        int CropHeight,
        int ControlLeft,
        int ControlTop,
        int ControlRight,
        int ControlBottom,
        int BackgroundX,
        int BackgroundY,
        Windows.Foundation.Rect LocalBoundsDip);

    /// <summary>按每个槽位中的控件边界 + 16 DIP 出血边距计算紧凑裁剪框。</summary>
    private List<SlotPixelRegion> BuildSlotRegions(double scale, int sheetWidth, int sheetHeight)
    {
        var regions = new List<SlotPixelRegion>(_activeSlots.Count);
        var bleedPx = (int)Math.Ceiling(Catalog.BleedMarginDip * scale);

        foreach (var (_, slotHost, control) in _activeSlots)
        {
            var boundsInHost = control
                .TransformToVisual(Host)
                .TransformBounds(new Windows.Foundation.Rect(0, 0, control.ActualWidth, control.ActualHeight));

            var ctrlLeftPx = (int)Math.Floor(boundsInHost.X * scale);
            var ctrlTopPx = (int)Math.Floor(boundsInHost.Y * scale);
            var ctrlRightPx = (int)Math.Ceiling((boundsInHost.X + boundsInHost.Width) * scale);
            var ctrlBottomPx = (int)Math.Ceiling((boundsInHost.Y + boundsInHost.Height) * scale);

            var slotBoundsInHost = slotHost
                .TransformToVisual(Host)
                .TransformBounds(new Windows.Foundation.Rect(0, 0, slotHost.ActualWidth, slotHost.ActualHeight));
            var slotMinX = Math.Max(0, (int)Math.Floor(slotBoundsInHost.X * scale));
            var slotMinY = Math.Max(0, (int)Math.Floor(slotBoundsInHost.Y * scale));
            var slotMaxX = Math.Min(sheetWidth, (int)Math.Ceiling((slotBoundsInHost.X + slotBoundsInHost.Width) * scale));
            var slotMaxY = Math.Min(sheetHeight, (int)Math.Ceiling((slotBoundsInHost.Y + slotBoundsInHost.Height) * scale));

            // 紧凑裁剪框：限制在当前槽位内，防止相邻状态槽位互相串入视野。
            var cropX = Math.Clamp(ctrlLeftPx - bleedPx, slotMinX, Math.Max(slotMinX, sheetWidth - 1));
            var cropY = Math.Clamp(ctrlTopPx - bleedPx, slotMinY, Math.Max(slotMinY, sheetHeight - 1));
            var cropRight = Math.Clamp(ctrlRightPx + bleedPx, cropX + 1, Math.Max(cropX + 1, slotMaxX));
            var cropBottom = Math.Clamp(ctrlBottomPx + bleedPx, cropY + 1, Math.Max(cropY + 1, slotMaxY));
            var cropWidth = cropRight - cropX;
            var cropHeight = cropBottom - cropY;

            var localBoundsDip = new Windows.Foundation.Rect(
                boundsInHost.X - (cropX / scale),
                boundsInHost.Y - (cropY / scale),
                boundsInHost.Width,
                boundsInHost.Height);

            regions.Add(new SlotPixelRegion(
                cropX,
                cropY,
                cropWidth,
                cropHeight,
                ctrlLeftPx,
                ctrlTopPx,
                ctrlRightPx,
                ctrlBottomPx,
                Math.Clamp(slotMinX, 0, sheetWidth - 1),
                Math.Clamp(slotMinY, 0, sheetHeight - 1),
                localBoundsDip));
        }

        return regions;
    }

    /// <summary>
    /// 在 controlBounds 像素范围内断言至少存在一个与槽位背景不同的像素。
    /// 控件边界越出图像时直接判失败，避免把被裁碎的控件当成有效参考。
    /// </summary>
    private static bool HasNonBackgroundPixels(
        byte[] pixels,
        int width,
        int height,
        SlotPixelRegion region)
    {
        if (region.ControlLeft < 0 ||
            region.ControlTop < 0 ||
            region.ControlRight > width ||
            region.ControlBottom > height ||
            region.ControlRight <= region.ControlLeft ||
            region.ControlBottom <= region.ControlTop)
        {
            return false;
        }

        var backgroundOffset = (region.BackgroundY * width + region.BackgroundX) * 4;
        var backgroundBlue = pixels[backgroundOffset];
        var backgroundGreen = pixels[backgroundOffset + 1];
        var backgroundRed = pixels[backgroundOffset + 2];

        for (var y = region.ControlTop; y < region.ControlBottom; y++)
        {
            var rowOffset = y * width;
            for (var x = region.ControlLeft; x < region.ControlRight; x++)
            {
                var offset = (rowOffset + x) * 4;
                if (Math.Abs(pixels[offset] - backgroundBlue) > 2 ||
                    Math.Abs(pixels[offset + 1] - backgroundGreen) > 2 ||
                    Math.Abs(pixels[offset + 2] - backgroundRed) > 2)
                {
                    return true;
                }
            }
        }

        return false;
    }

    private static byte[] CropBgraRegion(
        byte[] source,
        int sourceWidth,
        int sourceHeight,
        int x,
        int y,
        int width,
        int height)
    {
        var dst = new byte[width * height * 4];
        for (var row = 0; row < height; row++)
        {
            var srcY = Math.Clamp(y + row, 0, sourceHeight - 1);
            var srcOffset = (srcY * sourceWidth + x) * 4;
            var dstOffset = row * width * 4;
            System.Buffer.BlockCopy(source, srcOffset, dst, dstOffset, width * 4);
        }

        return dst;
    }

    private static async Task WriteBgraPngAsync(string pngPath, int width, int height, byte[] pixelBytes)
    {
        using var encodedStream = new InMemoryRandomAccessStream();
        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, encodedStream);
        encoder.SetPixelData(
            BitmapPixelFormat.Bgra8,
            BitmapAlphaMode.Premultiplied,
            (uint)width,
            (uint)height,
            96,
            96,
            pixelBytes);
        await encoder.FlushAsync();

        var pngBytes = await ReadStreamAsync(encodedStream);
        await File.WriteAllBytesAsync(pngPath, pngBytes);
    }

    private void ResizeClientAreaForSheet()
    {
        try
        {
            var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;
            _targetWidthDip = CanvasWidthDip;
            _targetHeightDip = CanvasHeightDip;
            var targetWidth = Math.Clamp((int)Math.Ceiling(_targetWidthDip * scale), 260, 8192);
            var targetHeight = Math.Clamp((int)Math.Ceiling(_targetHeightDip * scale), 96, 8192);
            AppWindow.ResizeClient(new SizeInt32(targetWidth, targetHeight));
        }
        catch (Exception ex)
        {
            _targetWidthDip = Math.Max(1.0, Host.ActualWidth);
            _targetHeightDip = Math.Max(1.0, Host.ActualHeight);
            Fatal.Report($"Failed to resize client area: {ex.Message}");
        }
    }

    private async Task WriteMetaAsync(
        string outputDirectory,
        string baseName,
        CaptureCase captureCase,
        int imagePixelWidth,
        int imagePixelHeight,
        Windows.Foundation.Rect localBounds,
        FrameworkElement control)
    {
        var appSdkAssembly = typeof(Application).Assembly.GetName();
        var sdkVersion = appSdkAssembly.Version?.ToString() ?? "unknown";
        var osVersion = Environment.OSVersion.Version.ToString();
        var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;
        var canonical = Catalog.CanonicalName(captureCase.ControlName);
        var fingerprint = Catalog.ComputeFingerprint(
            canonical,
            captureCase.State,
            captureCase.Theme,
            captureCase.Backdrop,
            scale,
            sdkVersion,
            osVersion);

        var meta = new
        {
            capturedAtUtc = DateTime.UtcNow.ToString("O", CultureInfo.InvariantCulture),
            control = canonical,
            state = captureCase.State.ToLowerInvariant(),
            theme = captureCase.Theme,
            backdrop = captureCase.Backdrop,
            fingerprint,
            systemAccent = Catalog.SystemAccentSignature(),
            pinnedAccent = Catalog.PinnedAccentColor,
            explicitWidth = Catalog.HasExplicitWidth(canonical),
            processBitness = Environment.Is64BitProcess ? "64-bit" : "32-bit",
            processArchitecture = RuntimeInformation.ProcessArchitecture.ToString(),
            osVersion = RuntimeInformation.OSDescription,
            osVersionNumber = osVersion,
            windowsAppSdkAssemblyVersion = sdkVersion,
            windowsAppSdkAssemblyFullName = appSdkAssembly.FullName,
            rasterizationScale = scale,
            hostActualWidth = Host.ActualWidth,
            hostActualHeight = Host.ActualHeight,
            requestedWindowWidth = (int)CanvasWidthDip,
            requestedWindowHeight = (int)CanvasHeightDip,
            imagePixelWidth,
            imagePixelHeight,
            controlBounds = new
            {
                x = Math.Round(localBounds.X, 2),
                y = Math.Round(localBounds.Y, 2),
                width = Math.Round(localBounds.Width, 2),
                height = Math.Round(localBounds.Height, 2),
            },
            controlFocusState = control.FocusState.ToString(),
            pngFile = $"{baseName}.png",
        };

        var json = JsonSerializer.Serialize(meta, new JsonSerializerOptions { WriteIndented = true });
        await File.WriteAllTextAsync(Path.Combine(outputDirectory, $"{baseName}.json"), json);
    }

    private static async Task<byte[]> ReadStreamAsync(IRandomAccessStream stream)
    {
        stream.Seek(0);
        var bytes = new byte[checked((int)stream.Size)];
        using var reader = new DataReader(stream.GetInputStreamAt(0));
        await reader.LoadAsync((uint)stream.Size);
        reader.ReadBytes(bytes);
        return bytes;
    }
}
