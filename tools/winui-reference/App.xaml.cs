using System;
using System.IO;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media.Animation;

namespace WinUiReference;

/// <summary>
/// 最小 WinUI 3 宿主：解析命令行参数后打开截图窗口。
/// </summary>
public partial class App : Application
{
    private Window? _window;

    public App()
    {
        // ProgressRing 是无限循环动画，截图相位不确定。关闭依赖动画后控件停留在
        // 初始静态状态，使每次采集得到同一帧。
        Timeline.AllowDependentAnimations = false;
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        if (!CaptureOptions.TryParse(Environment.GetCommandLineArgs(), out var options, out var error))
        {
            Fatal.Report(error);
            Environment.Exit(2);
            return;
        }

        _window = new CaptureWindow(options);
        _window.Activate();
    }
}

/// <summary>
/// 失败路径统一出口：控制台（可能被 WinExe 丢弃）、日志文件、退出码三条通道，
/// 方便自动化脚本定位问题。
/// </summary>
internal static class Fatal
{
    public static void Report(string message)
    {
        try
        {
            Console.Error.WriteLine(message);
        }
        catch
        {
            // WinExe 没有可用控制台时忽略
        }

        try
        {
            var logPath = Path.Combine(AppContext.BaseDirectory, "winui-reference.error.log");
            File.AppendAllText(logPath, $"[{DateTimeOffset.Now:O}] {message}{Environment.NewLine}");
        }
        catch
        {
            // 日志写不出去时只保留退出码
        }
    }
}
