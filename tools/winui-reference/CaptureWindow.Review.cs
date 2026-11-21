using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.Graphics;
using Windows.System;

namespace WinUiReference;

/// <summary>
/// 人工翻页检视模式（`--review`）的窗口控制层。
///
/// 与自动批采共用同一份 `PlanPages` 结果、同一棵可视树与同一张 1920x1080 画布：
/// 检视条放在 `Host` 之外，`Host` 仍是显式 1920x1080，因此「看到的」与「被采到的」
/// 是同一帧，切片与差分链路一行都不用改。
/// </summary>
public sealed partial class CaptureWindow
{
    /// <summary>检视条高度（DIP）：窗口让位给它的部分不参与画布布局。</summary>
    private const double ReviewBarHeightDip = 46.0;

    /// <summary>检视条按钮触发的导出正在进行中。</summary>
    private bool _reviewBusy;

    /// <summary>当前是否按窗口大小缩放显示（false = 1:1 实际大小）。</summary>
    private bool _fitToWindow = true;

    /// <summary>正在同步下拉框选中项，避免触发跳转。</summary>
    private bool _syncingJump;

    private void InitializeReviewUi()
    {
        foreach (var name in Catalog.ControlNames)
        {
            JumpBox.Items.Add(name);
        }

        AddAccelerator(VirtualKey.PageDown, VirtualKeyModifiers.None, NextPage);
        AddAccelerator(VirtualKey.Right, VirtualKeyModifiers.None, NextPage);
        AddAccelerator(VirtualKey.PageUp, VirtualKeyModifiers.None, PreviousPage);
        AddAccelerator(VirtualKey.Left, VirtualKeyModifiers.None, PreviousPage);
        AddAccelerator(VirtualKey.Home, VirtualKeyModifiers.None, () => GoToSheet(0));
        AddAccelerator(VirtualKey.End, VirtualKeyModifiers.None, () => GoToSheet(_sheets.Count - 1));
        AddAccelerator(VirtualKey.S, VirtualKeyModifiers.Control, CaptureCurrentPage);
        AddAccelerator(VirtualKey.Escape, VirtualKeyModifiers.None, Close);
    }

    private void AddAccelerator(VirtualKey key, VirtualKeyModifiers modifiers, Action action)
    {
        var accelerator = new KeyboardAccelerator { Key = key, Modifiers = modifiers };
        accelerator.Invoked += (_, args) =>
        {
            args.Handled = true;
            action();
        };
        RootGrid.KeyboardAccelerators.Add(accelerator);
    }

    /// <summary>进入检视模式：按当前屏幕摆好窗口，并装载第 <paramref name="index"/> 页。</summary>
    private void EnterReview(int index)
    {
        if (_sheets.Count == 0)
        {
            Fatal.Report("Nothing to review: the candidate matrix is empty.");
            Environment.ExitCode = 2;
            Application.Current.Exit();
            return;
        }

        ResizeClientForReview();
        GoToSheet(index);
    }

    /// <summary>切换到第 <paramref name="index"/> 页（越界即忽略，不做环绕）。</summary>
    private void GoToSheet(int index)
    {
        if (index < 0 || index >= _sheets.Count)
        {
            return;
        }

        _sheetIndex = index;
        if (!LoadSheet(ActiveSheet))
        {
            return;
        }

        // 检视条跟随页面主题，避免深色页顶出一条刺眼的浅色条。
        ReviewBar.RequestedTheme = Host.RequestedTheme;
        UpdateReviewBar();
    }

    private void NextPage() => GoToSheet(_sheetIndex + 1);

    private void PreviousPage() => GoToSheet(_sheetIndex - 1);

    private void UpdateReviewBar()
    {
        var sheet = ActiveSheet;

        PageLabel.Text =
            $"Page {_sheetIndex + 1} / {_sheets.Count} · {sheet.Theme} · {sheet.Backdrop} · {sheet.Slots.Count} cases";
        ToolTipService.SetToolTip(PageLabel, string.Join(", ", sheet.ControlNames));

        PrevPageButton.IsEnabled = !_reviewBusy && _sheetIndex > 0;
        NextPageButton.IsEnabled = !_reviewBusy && _sheetIndex + 1 < _sheets.Count;
        JumpBox.IsEnabled = !_reviewBusy;
        CapturePageButton.IsEnabled = !_reviewBusy;

        // 整页命中指纹时明确标出来：这一页 `just parity diff` 会直接复用，不会重渲。
        var cached = !_options.Refresh && AllCasesCached(sheet, CreateFingerprintContext());
        CachedBadge.Visibility = cached ? Visibility.Visible : Visibility.Collapsed;

        _syncingJump = true;
        var currentControl = sheet.ControlNames.FirstOrDefault();
        JumpBox.SelectedItem = currentControl is not null && JumpBox.Items.Contains(currentControl)
            ? currentControl
            : null;
        _syncingJump = false;

        CapturePageButton.Content = _reviewBusy ? "Exporting…" : "Capture page";
    }

    /// <summary>
    /// 把窗口收进当前显示器工作区：整页可见、按比例缩小显示。
    /// 缩小只发生在显示层（Viewbox），画布本身仍是 1920x1080；导出前会再把窗口
    /// 钉回 1:1 尺寸，保证写出的像素与自动批采完全一致。
    /// </summary>
    private void ResizeClientForReview()
    {
        try
        {
            var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;
            var area = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
            var barHeight = ReviewBarHeightDip * scale;
            var canvasWidth = CanvasWidthDip * scale;
            var canvasHeight = CanvasHeightDip * scale;

            var availableWidth = Math.Max(320.0, area.Width - 64.0);
            var availableHeight = Math.Max(240.0, area.Height - 96.0 - barHeight);
            var fit = Math.Min(1.0, Math.Min(availableWidth / canvasWidth, availableHeight / canvasHeight));

            _fitToWindow = true;
            HostScaler.Stretch = Stretch.Uniform;
            HostScroller.HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled;
            HostScroller.VerticalScrollBarVisibility = ScrollBarVisibility.Disabled;
            HostScroller.HorizontalScrollMode = ScrollMode.Disabled;
            HostScroller.VerticalScrollMode = ScrollMode.Disabled;
            ScaleToggleButton.Content = "Actual size";

            var width = (int)Math.Ceiling(canvasWidth * fit);
            var height = (int)Math.Ceiling(canvasHeight * fit + barHeight);
            AppWindow.ResizeClient(new SizeInt32(width, height));
            AppWindow.Move(new PointInt32(
                Math.Max(area.X, area.X + (area.Width - width) / 2),
                Math.Max(area.Y, area.Y + (area.Height - height) / 2)));
        }
        catch (Exception ex)
        {
            Fatal.Report($"Could not fit the review window to the screen: {ex.Message}");
        }
    }

    /// <summary>切到 1:1 实际大小；窗口大于屏幕时由 ScrollViewer 提供滚动。</summary>
    private void UseActualSize()
    {
        try
        {
            var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;
            _fitToWindow = false;
            HostScaler.Stretch = Stretch.None;
            HostScroller.HorizontalScrollBarVisibility = ScrollBarVisibility.Auto;
            HostScroller.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
            HostScroller.HorizontalScrollMode = ScrollMode.Auto;
            HostScroller.VerticalScrollMode = ScrollMode.Auto;
            ScaleToggleButton.Content = "Fit window";
            AppWindow.ResizeClient(new SizeInt32(
                (int)Math.Ceiling(CanvasWidthDip * scale),
                (int)Math.Ceiling((CanvasHeightDip + ReviewBarHeightDip) * scale)));
        }
        catch (Exception ex)
        {
            Fatal.Report($"Could not switch the review window to actual size: {ex.Message}");
        }
    }

    /// <summary>
    /// 导出当前页：先冻结合成动画并把窗口钉回 1:1，再走与批采完全相同的切片路径，
    /// 最后恢复窗口尺寸。导出失败只报告，不关闭窗口。
    /// </summary>
    private async void CaptureCurrentPage()
    {
        if (_reviewBusy)
        {
            return;
        }

        _reviewBusy = true;
        UpdateReviewBar();

        var pinned = false;
        SizeInt32 restore = default;
        try
        {
            foreach (var (_, _, control) in _activeSlots)
            {
                FreezeAnimations(control);
            }

            pinned = TryPinClientToCaptureCanvas(out restore);
            await Task.Delay(200);
            Host.UpdateLayout();
            await CaptureSheetAsync(advance: false, forceSheet: true);
        }
        catch (Exception ex)
        {
            Fatal.Report($"Exporting the current page failed: {ex}");
        }
        finally
        {
            if (pinned)
            {
                try
                {
                    AppWindow.ResizeClient(restore);
                }
                catch (Exception ex)
                {
                    Fatal.Report($"Could not restore the review window size: {ex.Message}");
                }
            }

            _reviewBusy = false;
            UpdateReviewBar();
        }
    }

    /// <summary>把客户区钉到 1920x1080 DIP 的 1:1 尺寸，并记住原尺寸以便还原。</summary>
    private bool TryPinClientToCaptureCanvas(out SizeInt32 restore)
    {
        restore = default;
        try
        {
            restore = AppWindow.ClientSize;
            var scale = Host.XamlRoot?.RasterizationScale ?? 1.0;
            AppWindow.ResizeClient(new SizeInt32(
                (int)Math.Ceiling(CanvasWidthDip * scale),
                (int)Math.Ceiling(CanvasHeightDip * scale)));
            return true;
        }
        catch (Exception ex)
        {
            Fatal.Report($"Could not pin the client area to the capture canvas: {ex.Message}");
            return false;
        }
    }

    private void OnPrevPageClick(object sender, RoutedEventArgs e) => PreviousPage();

    private void OnNextPageClick(object sender, RoutedEventArgs e) => NextPage();

    private void OnJumpSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_syncingJump || JumpBox.SelectedItem is not string control)
        {
            return;
        }

        for (var index = 0; index < _sheets.Count; index++)
        {
            if (_sheets[index].ControlNames.Contains(control, StringComparer.OrdinalIgnoreCase))
            {
                GoToSheet(index);
                return;
            }
        }
    }

    private void OnCapturePageClick(object sender, RoutedEventArgs e) => CaptureCurrentPage();

    private void OnScaleToggleClick(object sender, RoutedEventArgs e)
    {
        if (_fitToWindow)
        {
            UseActualSize();
        }
        else
        {
            ResizeClientForReview();
        }
    }

    private void OnOpenDirectoryClick(object sender, RoutedEventArgs e)
    {
        try
        {
            var directory = Path.GetFullPath(_options.OutputDirectory);
            Directory.CreateDirectory(directory);
            Process.Start(new ProcessStartInfo
            {
                FileName = "explorer.exe",
                Arguments = directory,
                UseShellExecute = true,
            });
        }
        catch (Exception ex)
        {
            Fatal.Report($"Could not open the capture directory: {ex.Message}");
        }
    }
}
