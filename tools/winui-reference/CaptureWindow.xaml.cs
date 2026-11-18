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
/// 截图窗口：按「控件 × 主题 × 后景」为单位构建状态拼版（Contact Sheet），
/// 将同一控件的多个状态并排放入同一张网格画布，单次等待 350ms 动画收敛、单次导出大图，
/// 再按各槽位的紧凑出血区域切分落盘，并支持基于版本与规格指纹的增量缓存。
/// </summary>
public sealed partial class CaptureWindow : Window
{
    private readonly CaptureOptions _options;
    private readonly IReadOnlyList<ContactSheetBatch> _sheets;
    private readonly System.Diagnostics.Stopwatch _settle = new();
    private int _sheetIndex;
    private readonly List<(CaptureCase Case, Border SlotHost, FrameworkElement Control)> _activeSlots = new();
    private DispatcherQueueTimer? _timeoutTimer;
    private int _renderingTicks;
    private int _captureStarted;
    private bool _loaded;
    private int _skippedCount;
    private int _capturedCount;

    public CaptureWindow(CaptureOptions options)
    {
        _options = options;
        InitializeComponent();

        _sheets = options.BuildSheets();
        Title = $"WinUI Reference - {options.ControlName}";

        Host.Loaded += OnHostLoaded;
    }

    private ContactSheetBatch ActiveSheet => _sheets[_sheetIndex];

    private void OnHostLoaded(object sender, RoutedEventArgs e)
    {
        if (_loaded)
        {
            return;
        }

        _loaded = true;
        AdvanceToNextUncachedOrStart(initial: true);
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

        while (_sheetIndex < _sheets.Count)
        {
            var sheet = _sheets[_sheetIndex];
            if (!_options.NoCache && AllCasesCached(sheet))
            {
                _skippedCount += sheet.Cases.Count;
                Console.Out.WriteLine(
                    $"[cache-hit] 跳过 {sheet.ControlName} ({sheet.Theme}, {sheet.Backdrop}) 的 {sheet.Cases.Count} 个状态");
                _sheetIndex++;
                continue;
            }

            if (!LoadSheet(sheet))
            {
                Environment.ExitCode = 2;
                Application.Current.Exit();
                return;
            }

            ResizeClientAreaForSheet(sheet.Cases.Count);
            StartCaptureLoop();
            return;
        }

        Console.Out.WriteLine($"采集完成：新增 {_capturedCount} 个用例，缓存命中跳过 {_skippedCount} 个用例。");
        Application.Current.Exit();
    }

    /// <summary>检查当前拼版中的所有用例是否均在磁盘上拥有指纹匹配的 PNG 与 JSON。</summary>
    private bool AllCasesCached(ContactSheetBatch sheet)
    {
        var outputDirectory = Path.GetFullPath(_options.OutputDirectory);
        var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;
        var sdkVersion = typeof(Application).Assembly.GetName().Version?.ToString() ?? "unknown";
        var osVersion = Environment.OSVersion.Version.ToString();

        foreach (var c in sheet.Cases)
        {
            var baseName = c.FileBaseName();
            var pngPath = Path.Combine(outputDirectory, $"{baseName}.png");
            var jsonPath = Path.Combine(outputDirectory, $"{baseName}.json");
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
                    scale,
                    sdkVersion,
                    osVersion);
                if (!jsonText.Contains($"\"fingerprint\": \"{expectedFp}\"", StringComparison.Ordinal))
                {
                    return false;
                }
            }
            catch
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>
    /// 把一个「控件 × 主题 × 后景」下的全部待测状态并排装载进 Host 的水平网格槽位中。
    /// </summary>
    private bool LoadSheet(ContactSheetBatch sheet)
    {
        Host.RequestedTheme = sheet.Theme == "dark" ? ElementTheme.Dark : ElementTheme.Light;
        Host.Background = Catalog.CreateBackdropBrush(sheet.Theme, sheet.Backdrop);
        Host.Children.Clear();
        Host.ColumnDefinitions.Clear();
        Host.RowDefinitions.Clear();
        _activeSlots.Clear();

        for (var idx = 0; idx < sheet.Cases.Count; idx++)
        {
            var captureCase = sheet.Cases[idx];
            Host.ColumnDefinitions.Add(new ColumnDefinition
            {
                Width = new GridLength(Catalog.SlotWidthDip, GridUnitType.Pixel),
            });

            if (!Catalog.TryCreate(captureCase.ControlName, out var control))
            {
                Fatal.Report($"未知控件：{captureCase.ControlName}{Environment.NewLine}可用控件：{string.Join(", ", Catalog.ControlNames)}");
                return false;
            }

            if (!Catalog.TryApplyState(control, captureCase.State, out var stateError))
            {
                Fatal.Report(stateError);
                return false;
            }

            var slotHost = new Border
            {
                Width = Catalog.SlotWidthDip,
                Height = Catalog.SlotHeightDip,
                Background = Catalog.CreateBackdropBrush(sheet.Theme, sheet.Backdrop),
                Child = control,
            };
            Grid.SetColumn(slotHost, idx);
            Host.Children.Add(slotHost);
            _activeSlots.Add((captureCase, slotHost, control));
        }

        _renderingTicks = 0;
        _settle.Restart();
        _captureStarted = 0;
        return true;
    }

    private void StartCaptureLoop()
    {
        CompositionTarget.Rendering += OnCompositionRendering;

        // 兜底：远程桌面/最小化等场景可能收不到渲染回调，超时后仍然尝试截图。
        _timeoutTimer = DispatcherQueue.CreateTimer();
        _timeoutTimer.Interval = TimeSpan.FromSeconds(10);
        _timeoutTimer.IsRepeating = false;
        _timeoutTimer.Tick += (_, _) => BeginCapture("渲染事件超时（10 秒），改为直接截图");
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

        // 同一拼版内的所有状态共享这一次 350ms VisualState 动画收敛等待。
        if (_renderingTicks < 3 || _settle.ElapsedMilliseconds < 350)
        {
            return;
        }

        BeginCapture(warning: null);
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

        _ = CaptureSheetAsync();
    }

    private async Task CaptureSheetAsync()
    {
        try
        {
            var outputDirectory = Path.GetFullPath(_options.OutputDirectory);
            Directory.CreateDirectory(outputDirectory);

            // 单次 RenderTargetBitmap 渲染整张状态拼版
            var bitmap = new RenderTargetBitmap();
            await bitmap.RenderAsync(Host);

            var pixelBuffer = await bitmap.GetPixelsAsync();
            var sheetPixels = new byte[checked((int)pixelBuffer.Length)];
            using (var pixelReader = DataReader.FromBuffer(pixelBuffer))
            {
                pixelReader.ReadBytes(sheetPixels);
            }

            var sheetWidth = bitmap.PixelWidth;
            var sheetHeight = bitmap.PixelHeight;
            var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;

            // 可选保留拼版总览图供人工速览
            if (_options.SaveContactSheet)
            {
                var sheetName = $"{Catalog.CanonicalName(ActiveSheet.ControlName)}_{ActiveSheet.Theme}_{ActiveSheet.Backdrop}_sheet.png";
                var sheetPath = Path.Combine(outputDirectory, sheetName);
                await WriteBgraPngAsync(sheetPath, sheetWidth, sheetHeight, sheetPixels);
            }

            // 按每个槽位中的控件边界 + 16 DIP 出血边距切出紧凑单图与同名 JSON
            foreach (var (captureCase, slotHost, control) in _activeSlots)
            {
                var boundsInHost = control
                    .TransformToVisual(Host)
                    .TransformBounds(new Windows.Foundation.Rect(0, 0, control.ActualWidth, control.ActualHeight));

                var bleedPx = (int)Math.Ceiling(Catalog.BleedMarginDip * scale);
                var ctrlLeftPx = (int)Math.Floor(boundsInHost.X * scale);
                var ctrlTopPx = (int)Math.Floor(boundsInHost.Y * scale);
                var ctrlRightPx = (int)Math.Ceiling((boundsInHost.X + boundsInHost.Width) * scale);
                var ctrlBottomPx = (int)Math.Ceiling((boundsInHost.Y + boundsInHost.Height) * scale);

                var slotBoundsInHost = slotHost
                    .TransformToVisual(Host)
                    .TransformBounds(new Windows.Foundation.Rect(0, 0, slotHost.ActualWidth, slotHost.ActualHeight));
                var slotMinX = Math.Max(0, (int)Math.Floor(slotBoundsInHost.X * scale));
                var slotMaxX = Math.Min(sheetWidth, (int)Math.Ceiling((slotBoundsInHost.X + slotBoundsInHost.Width) * scale));

                // 紧凑裁剪框：限制在当前槽位内，防止相邻状态槽位互相串入视野
                var cropX = Math.Clamp(ctrlLeftPx - bleedPx, slotMinX, Math.Max(slotMinX, sheetWidth - 1));
                var cropY = Math.Clamp(ctrlTopPx - bleedPx, 0, Math.Max(0, sheetHeight - 1));
                var cropRight = Math.Clamp(ctrlRightPx + bleedPx, cropX + 1, Math.Max(cropX + 1, slotMaxX));
                var cropBottom = Math.Clamp(ctrlBottomPx + bleedPx, cropY + 1, Math.Max(cropY + 1, sheetHeight));
                var cropWidth = cropRight - cropX;
                var cropHeight = cropBottom - cropY;

                var croppedBytes = CropBgraRegion(sheetPixels, sheetWidth, sheetHeight, cropX, cropY, cropWidth, cropHeight);

                var baseName = captureCase.FileBaseName();
                var pngPath = Path.Combine(outputDirectory, $"{baseName}.png");
                await WriteBgraPngAsync(pngPath, cropWidth, cropHeight, croppedBytes);

                // 控件在裁剪后紧凑画布内的相对 DIP 坐标
                var localBoundsDip = new Windows.Foundation.Rect(
                    boundsInHost.X - (cropX / scale),
                    boundsInHost.Y - (cropY / scale),
                    boundsInHost.Width,
                    boundsInHost.Height);

                await WriteMetaAsync(
                    outputDirectory,
                    baseName,
                    captureCase,
                    cropWidth,
                    cropHeight,
                    localBoundsDip,
                    control);

                _capturedCount++;
                Console.Out.WriteLine($"已写出 {pngPath} ({cropWidth}x{cropHeight})");
            }
        }
        catch (Exception ex)
        {
            Fatal.Report($"截图失败：{ex}");
            Environment.ExitCode = 1;
            Application.Current.Exit();
            return;
        }

        AdvanceToNextUncachedOrStart(initial: false);
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

    private void ResizeClientAreaForSheet(int slotCount)
    {
        try
        {
            var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;
            var targetWidth = _options.Batch
                ? Math.Clamp((int)Math.Ceiling(Catalog.SlotWidthDip * Math.Max(1, slotCount) * scale), 260, 8192)
                : _options.Width;
            var targetHeight = _options.Batch
                ? Math.Clamp((int)Math.Ceiling(Catalog.SlotHeightDip * scale), 96, 8192)
                : _options.Height;
            AppWindow.ResizeClient(new SizeInt32(targetWidth, targetHeight));
        }
        catch (Exception ex)
        {
            Fatal.Report($"调整窗口大小失败：{ex.Message}");
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
            requestedWindowWidth = _options.Width,
            requestedWindowHeight = _options.Height,
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

/// <summary>一个截图用例（四维坐标：控件 × 状态 × 主题 × 后景底板）。</summary>
public sealed class CaptureCase
{
    public CaptureCase(string controlName, string state, string theme, string backdrop = "solid")
    {
        ControlName = controlName;
        State = state;
        Theme = theme;
        Backdrop = backdrop;
    }

    public string ControlName { get; }

    public string State { get; }

    public string Theme { get; }

    public string Backdrop { get; }

    /// <summary>
    /// 生成输出文件基名：默认 solid 后景保持与既有 `<Control>_<state>_<theme>` 完全兼容；
    /// 非 solid 后景追加 `_<backdrop>` 后缀。
    /// </summary>
    public string FileBaseName()
    {
        var canonical = Catalog.CanonicalName(ControlName);
        var state = State.ToLowerInvariant();
        return Backdrop == "solid"
            ? $"{canonical}_{state}_{Theme}"
            : $"{canonical}_{state}_{Theme}_{Backdrop}";
    }
}

/// <summary>按「控件 × 主题 × 后景」组织的单张状态拼版批次。</summary>
public sealed class ContactSheetBatch
{
    public ContactSheetBatch(string controlName, string theme, string backdrop, IReadOnlyList<CaptureCase> cases)
    {
        ControlName = controlName;
        Theme = theme;
        Backdrop = backdrop;
        Cases = cases;
    }

    public string ControlName { get; }

    public string Theme { get; }

    public string Backdrop { get; }

    public IReadOnlyList<CaptureCase> Cases { get; }
}

/// <summary>
/// 命令行参数。所有参数都是可选的，缺省值用于快速产出基准截图。
/// </summary>
public sealed class CaptureOptions
{
    public const int DefaultWidth = 420;
    public const int DefaultHeight = 160;
    public const string DefaultOutputDirectory = "captures";

    private const string Usage =
        "用法: WinUiReference --out <目录> [--control <名称>] [--state normal|disabled|checked|focused] " +
        "[--theme light|dark] [--backdrop solid|mica|mica_alt|acrylic|all] " +
        "[--width <像素>] [--height <像素>] [--batch] [--no-cache] [--save-sheet]";

    public string OutputDirectory { get; private init; } = DefaultOutputDirectory;

    public string ControlName { get; private init; } = "Button";

    public string State { get; private init; } = "normal";

    public string Theme { get; private init; } = "light";

    public string Backdrop { get; private init; } = "solid";

    public int Width { get; private init; } = DefaultWidth;

    public int Height { get; private init; } = DefaultHeight;

    /// <summary>批量模式：按状态拼版跑完所有试点组合。</summary>
    public bool Batch { get; private init; }

    /// <summary>禁用指纹缓存，强制重新截图。</summary>
    public bool NoCache { get; private init; }

    /// <summary>同时落盘未切分的 `_sheet.png` 拼版大图。</summary>
    public bool SaveContactSheet { get; private init; }

    /// <summary>展开本次运行要采集的全部状态拼版批次。</summary>
    public IReadOnlyList<ContactSheetBatch> BuildSheets()
    {
        if (!Batch)
        {
            var single = new CaptureCase(ControlName, State, Theme, Backdrop);
            return new[]
            {
                new ContactSheetBatch(ControlName, Theme, Backdrop, new[] { single }),
            };
        }

        var backdrops = Backdrop == "all"
            ? Catalog.BackdropKinds
            : new[] { Backdrop };

        var sheets = new List<ContactSheetBatch>();
        foreach (var backdrop in backdrops)
        {
            foreach (var control in Catalog.ControlNames)
            {
                foreach (var theme in new[] { "light", "dark" })
                {
                    var states = new List<CaptureCase>();
                    foreach (var state in Catalog.ImplementedStates)
                    {
                        if (state == "focused" || !Catalog.SupportsState(control, state))
                        {
                            continue;
                        }

                        states.Add(new CaptureCase(control, state, theme, backdrop));
                    }

                    if (states.Count > 0)
                    {
                        sheets.Add(new ContactSheetBatch(control, theme, backdrop, states));
                    }
                }
            }
        }

        return sheets;
    }

    public static bool TryParse(string[] args, out CaptureOptions options, out string error)
    {
        options = new CaptureOptions();
        error = string.Empty;

        var outputDirectory = DefaultOutputDirectory;
        var controlName = "Button";
        var state = "normal";
        var theme = "light";
        var backdrop = "solid";
        var width = DefaultWidth;
        var height = DefaultHeight;
        var batch = false;
        var noCache = false;
        var saveSheet = false;

        for (var i = 1; i < args.Length; i++)
        {
            var name = args[i];
            var value = i + 1 < args.Length && !args[i + 1].StartsWith("--", StringComparison.Ordinal)
                ? args[i + 1]
                : null;

            switch (name.ToLowerInvariant())
            {
                case "--batch":
                    batch = true;
                    break;

                case "--no-cache":
                    noCache = true;
                    break;

                case "--save-sheet":
                    saveSheet = true;
                    break;

                case "--out":
                    if (value is null)
                    {
                        error = UsageError("--out 缺少目录参数");
                        return false;
                    }

                    outputDirectory = value;
                    i++;
                    break;

                case "--control":
                    if (value is null)
                    {
                        error = UsageError("--control 缺少控件名");
                        return false;
                    }

                    controlName = value;
                    i++;
                    break;

                case "--state":
                    if (value is null)
                    {
                        error = UsageError("--state 缺少状态名");
                        return false;
                    }

                    state = value.ToLowerInvariant();
                    i++;
                    break;

                case "--theme":
                    if (value is null)
                    {
                        error = UsageError("--theme 缺少主题名");
                        return false;
                    }

                    theme = value.ToLowerInvariant();
                    i++;
                    break;

                case "--backdrop":
                    if (value is null)
                    {
                        error = UsageError("--backdrop 缺少后景类型");
                        return false;
                    }

                    backdrop = value.ToLowerInvariant();
                    i++;
                    break;

                case "--width":
                    if (!TryParseSize(value, out width))
                    {
                        error = UsageError("--width 需要整数像素");
                        return false;
                    }

                    i++;
                    break;

                case "--height":
                    if (!TryParseSize(value, out height))
                    {
                        error = UsageError("--height 需要整数像素");
                        return false;
                    }

                    i++;
                    break;

                default:
                    error = UsageError($"未知参数：{name}");
                    return false;
            }
        }

        if (backdrop != "all" && !Catalog.BackdropKinds.Contains(backdrop, StringComparer.OrdinalIgnoreCase))
        {
            error = UsageError($"未知后景类型：{backdrop}，可用值：{string.Join(", ", Catalog.BackdropKinds)}, all");
            return false;
        }

        if (!batch)
        {
            if (backdrop == "all")
            {
                error = UsageError("单用例模式不支持 --backdrop all，请指定具体后景或加 --batch");
                return false;
            }

            if (!Catalog.ControlNames.Contains(controlName, StringComparer.OrdinalIgnoreCase))
            {
                error = UsageError($"未知控件：{controlName}{Environment.NewLine}可用控件：{string.Join(", ", Catalog.ControlNames)}");
                return false;
            }

            if (!Catalog.ImplementedStates.Contains(state, StringComparer.OrdinalIgnoreCase))
            {
                error = Catalog.ReservedStates.Contains(state, StringComparer.OrdinalIgnoreCase)
                    ? UsageError($"状态 {state} 尚未实现（仅保留枚举），请改用 {string.Join("/", Catalog.ImplementedStates)}")
                    : UsageError($"未知状态：{state}，可用状态：{string.Join(", ", Catalog.ImplementedStates)}");
                return false;
            }

            if (theme != "light" && theme != "dark")
            {
                error = UsageError($"未知主题：{theme}，可用主题：light, dark");
                return false;
            }
        }

        if (width is < 64 or > 8192 || height is < 64 or > 8192)
        {
            error = UsageError($"窗口尺寸必须在 64..8192 之间，当前为 {width}x{height}");
            return false;
        }

        if (string.IsNullOrWhiteSpace(outputDirectory))
        {
            error = UsageError("--out 不能为空");
            return false;
        }

        options = new CaptureOptions
        {
            OutputDirectory = outputDirectory,
            ControlName = Catalog.CanonicalName(controlName),
            State = state,
            Theme = theme,
            Backdrop = backdrop,
            Width = width,
            Height = height,
            Batch = batch,
            NoCache = noCache,
            SaveContactSheet = saveSheet,
        };

        return true;
    }

    private static bool TryParseSize(string? value, out int size)
    {
        size = 0;
        return value is not null && int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out size);
    }

    private static string UsageError(string message)
    {
        return $"{message}{Environment.NewLine}{Usage}";
    }
}
