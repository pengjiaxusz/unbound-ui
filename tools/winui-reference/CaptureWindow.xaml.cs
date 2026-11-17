using System;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace WinUiReference;

/// <summary>
/// 截图窗口：承载单个控件，等待渲染稳定后用 RenderTargetBitmap 导出 PNG 与 meta.json。
/// </summary>
public sealed partial class CaptureWindow : Window
{
    private readonly CaptureOptions _options;
    private FrameworkElement? _control;
    private DispatcherQueueTimer? _timeoutTimer;
    private int _renderingTicks;
    private int _captureStarted;
    private bool _loaded;

    public CaptureWindow(CaptureOptions options)
    {
        _options = options;
        InitializeComponent();

        Host.RequestedTheme = options.Theme == "dark" ? ElementTheme.Dark : ElementTheme.Light;
        Title = $"WinUI Reference - {options.ControlName}";
        Host.Loaded += OnHostLoaded;

        if (!Catalog.TryCreate(options.ControlName, out var control))
        {
            Fatal.Report($"未知控件：{options.ControlName}{Environment.NewLine}可用控件：{string.Join(", ", Catalog.ControlNames)}");
            Environment.Exit(2);
            return;
        }

        _control = control;
        Host.Children.Add(control);
    }

    private void OnHostLoaded(object sender, RoutedEventArgs e)
    {
        if (_loaded)
        {
            return;
        }

        _loaded = true;

        if (!Catalog.TryApplyState(_control!, _options.State, out var stateError))
        {
            Fatal.Report(stateError);
            Environment.Exit(2);
            return;
        }

        ResizeClientArea();

        // 焦点必须在 Loaded 之后申请，否则会被系统忽略。
        Catalog.ApplyFocus(_control!, _options.State);

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

        // 第 3 个渲染帧再截图：前两帧用于布局、主题切换与状态生效上屏。
        if (_renderingTicks < 3)
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

        _ = CaptureAsync();
    }

    private async Task CaptureAsync()
    {
        try
        {
            var outputDirectory = Path.GetFullPath(_options.OutputDirectory);
            Directory.CreateDirectory(outputDirectory);

            var control = _control!;
            var bitmap = new RenderTargetBitmap();
            await bitmap.RenderAsync(Host);

            var pixelBuffer = await bitmap.GetPixelsAsync();
            var pixelBytes = new byte[checked((int)pixelBuffer.Length)];
            using (var pixelReader = DataReader.FromBuffer(pixelBuffer))
            {
                pixelReader.ReadBytes(pixelBytes);
            }

            using var encodedStream = new InMemoryRandomAccessStream();
            var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, encodedStream);
            encoder.SetPixelData(
                BitmapPixelFormat.Bgra8,
                BitmapAlphaMode.Premultiplied,
                (uint)bitmap.PixelWidth,
                (uint)bitmap.PixelHeight,
                96,
                96,
                pixelBytes);
            await encoder.FlushAsync();

            var pngBytes = await ReadStreamAsync(encodedStream);
            var fileName = $"{Catalog.CanonicalName(_options.ControlName)}_{_options.State.ToLowerInvariant()}_{_options.Theme}.png";
            var pngPath = Path.Combine(outputDirectory, fileName);
            await File.WriteAllBytesAsync(pngPath, pngBytes);

            await WriteMetaAsync(outputDirectory, bitmap, control);
            Console.Out.WriteLine($"已写出 {pngPath}");
        }
        catch (Exception ex)
        {
            Fatal.Report($"截图失败：{ex}");
            Environment.ExitCode = 1;
        }
        finally
        {
            Application.Current.Exit();
        }
    }

    private void ResizeClientArea()
    {
        try
        {
            // ResizeClient 的单位是物理像素；Host 的 ActualWidth 是 DIP，二者之比即光栅化比例。
            AppWindow.ResizeClient(new SizeInt32(_options.Width, _options.Height));
        }
        catch (Exception ex)
        {
            Fatal.Report($"调整窗口大小失败：{ex.Message}");
        }
    }

    private async Task WriteMetaAsync(string outputDirectory, RenderTargetBitmap bitmap, FrameworkElement control)
    {
        var bounds = control
            .TransformToVisual(Host)
            .TransformBounds(new Windows.Foundation.Rect(0, 0, control.ActualWidth, control.ActualHeight));

        var appSdkAssembly = typeof(Microsoft.UI.Xaml.Application).Assembly.GetName();

        var meta = new
        {
            capturedAtUtc = DateTime.UtcNow.ToString("O", CultureInfo.InvariantCulture),
            control = Catalog.CanonicalName(_options.ControlName),
            state = _options.State.ToLowerInvariant(),
            theme = _options.Theme,
            processBitness = Environment.Is64BitProcess ? "64-bit" : "32-bit",
            processArchitecture = RuntimeInformation.ProcessArchitecture.ToString(),
            osVersion = RuntimeInformation.OSDescription,
            osVersionNumber = Environment.OSVersion.Version.ToString(),
            windowsAppSdkAssemblyVersion = appSdkAssembly.Version?.ToString(),
            windowsAppSdkAssemblyFullName = appSdkAssembly.FullName,
            rasterizationScale = Host.XamlRoot?.RasterizationScale ?? 1.0,
            hostActualWidth = Host.ActualWidth,
            hostActualHeight = Host.ActualHeight,
            requestedWindowWidth = _options.Width,
            requestedWindowHeight = _options.Height,
            imagePixelWidth = bitmap.PixelWidth,
            imagePixelHeight = bitmap.PixelHeight,
            controlBounds = new
            {
                x = bounds.X,
                y = bounds.Y,
                width = bounds.Width,
                height = bounds.Height,
            },
            controlFocusState = control.FocusState.ToString(),
            pngFile = $"{Catalog.CanonicalName(_options.ControlName)}_{_options.State.ToLowerInvariant()}_{_options.Theme}.png",
        };

        var json = JsonSerializer.Serialize(meta, new JsonSerializerOptions { WriteIndented = true });
        await File.WriteAllTextAsync(Path.Combine(outputDirectory, "meta.json"), json);
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
        "[--theme light|dark] [--width <像素>] [--height <像素>]";

    public string OutputDirectory { get; private init; } = DefaultOutputDirectory;

    public string ControlName { get; private init; } = "Button";

    public string State { get; private init; } = "normal";

    public string Theme { get; private init; } = "light";

    public int Width { get; private init; } = DefaultWidth;

    public int Height { get; private init; } = DefaultHeight;

    public static bool TryParse(string[] args, out CaptureOptions options, out string error)
    {
        options = new CaptureOptions();
        error = string.Empty;

        var outputDirectory = DefaultOutputDirectory;
        var controlName = "Button";
        var state = "normal";
        var theme = "light";
        var width = DefaultWidth;
        var height = DefaultHeight;

        for (var i = 1; i < args.Length; i++)
        {
            var name = args[i];
            var value = i + 1 < args.Length && !args[i + 1].StartsWith("--", StringComparison.Ordinal)
                ? args[i + 1]
                : null;

            switch (name.ToLowerInvariant())
            {
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
            Width = width,
            Height = height,
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
