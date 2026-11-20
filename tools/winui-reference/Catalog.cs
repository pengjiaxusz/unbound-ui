using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using System.Runtime.InteropServices.WindowsRuntime;
using Microsoft.UI.Xaml.Markup;
using Windows.UI;

namespace WinUiReference;

/// <summary>
/// 试点控件清单、后景底板配方与用例指纹计算。
/// 控件内容全部使用确定性文案与固定尺寸，避免系统语言、默认值或窗口宽度影响截图结果。
/// </summary>
internal static class Catalog
{
    /// <summary>
    /// 采集配方版本：槽位按 controlBounds 外扩出血自适应、resize 后再等待、
    /// 截图后校验非背景像素。配方变化时必须同步提升，确保旧缓存全部失效。
    /// </summary>
    public const string CaptureRecipeVersion = "v3-slot-bleed-content-contract";

    /// <summary>
    /// 采集宿主钉死的 WinUI 11 默认 Fluent 强调色（浅色 / 深色）。
    /// 与 unbound-ui 的 win11.rs 原生蓝阶梯一致，用于元数据显式记录基准，便于诊断系统强调色泄漏。
    /// </summary>
    public const string PinnedAccentColor = "#005FB8/#60CDFF";

    /// <summary>
    /// 采集机当前系统强调色签名（Accent / AccentLight1 / AccentDark1 的 RGB 十六进制）。
    /// 宿主虽在 App.xaml 与 CaptureWindow.xaml 覆盖了全部已知强调色资源键，但 WinUI 模板仍在演进，
    /// 未知资源键会把系统个性化强调色泄漏进参考图（历史案例：红色描边、紫色 CheckBox 选中描边）。
    /// 将系统强调色纳入指纹后，切换系统强调色必然使旧缓存失效并触发重采，
    /// 既消除「换色后仍复用旧图」，也让钉死蓝的宿主重新产出稳定的蓝色基准。
    /// </summary>
    private static readonly Lazy<string> SystemAccentValue = new(ComputeSystemAccentSignature);

    /// <summary>读取并缓存当前系统强调色签名，供指纹与元数据共用。</summary>
    public static string SystemAccentSignature() => SystemAccentValue.Value;

    private static string ComputeSystemAccentSignature()
    {
        try
        {
            var settings = new Windows.UI.ViewManagement.UISettings();
            string Hex(Windows.UI.Color color) => $"{color.R:X2}{color.G:X2}{color.B:X2}";
            return string.Join(",",
                Hex(settings.GetColorValue(Windows.UI.ViewManagement.UIColorType.Accent)),
                Hex(settings.GetColorValue(Windows.UI.ViewManagement.UIColorType.AccentLight1)),
                Hex(settings.GetColorValue(Windows.UI.ViewManagement.UIColorType.AccentDark1)));
        }
        catch
        {
            // 非打包环境或远程会话可能拒绝读取 UISettings；返回固定哨兵值，保证指纹确定性。
            return "unavailable";
        }
    }

    /// <summary>拼版槽位四周预留的阴影与焦点环出血边距（DIP）。</summary>
    public const int BleedMarginDip = 16;

    /// <summary>拼版图集单个槽位的标准宽度（DIP）。</summary>
    public const int SlotWidthDip = 260;

    /// <summary>拼版图集单个槽位的标准高度（DIP）。</summary>
    public const int SlotHeightDip = 96;

    private static readonly string[] ControlNameList =
    {
        // 首批试点（6）
        "Button", "CheckBox", "ToggleSwitch", "Slider", "ComboBox", "TextBox",
        // 第一批：基础输入与文本类（12）
        "ToggleButton", "SplitButton", "RadioButton", "RatingControl", "ColorPicker",
        "NumberBox", "AutoSuggestBox", "RichTextBlock", "ProgressBar", "ProgressRing",
        "InfoBadge", "ToolTip",
        // 第二批：导航、浮层、集合与日期类（28）
        "InfoBar", "TeachingTip", "ContentDialog", "Flyout", "MenuFlyout", "MenuBar",
        "CommandBar", "NavigationView", "TabView", "Pivot", "BreadcrumbBar", "SelectorBar",
        "PipsPager", "Expander", "SplitView", "Border", "Card", "StackPanel", "ScrollView",
        "TreeView", "ListView", "FlipView", "CalendarView", "TimePicker", "PersonPicture",
        "Image", "FontIcon", "TitleBar",
        // 第三批：后续补齐的 WinUI 3 控件（11）
        "RichEditBox", "GridView", "ListBox", "CommandBarFlyout", "AppBarButton",
        "AppBarToggleButton", "AppBarSeparator", "SwipeControl", "RefreshContainer",
        "AnnotatedScrollBar", "ItemsView",
        // 第四批：按钮与切换类补充（4）
        "HyperlinkButton", "RepeatButton", "DropDownButton", "ToggleSplitButton",
        // 第五批：输入与日期类补充（3）
        "PasswordBox", "DatePicker", "CalendarDatePicker",
    };

    private static readonly string[] ImplementedStateList = { "normal", "disabled", "checked", "focused" };

    // hover / pressed 只保留枚举，尚未实现（需要模拟指针状态，暂不纳入试点）。
    private static readonly string[] ReservedStateList = { "hover", "pressed" };

    /// <summary>
    /// 支持的后景合成底板类型：
    /// solid 为当前默认激活项；mica / mica_alt / acrylic 预留确定性合成底板，待云母管线完工后全量开启。
    /// </summary>
    private static readonly string[] BackdropKindList = { "solid", "mica", "mica_alt", "acrylic" };

    public static IReadOnlyList<string> ControlNames => ControlNameList;

    public static IReadOnlyList<string> ImplementedStates => ImplementedStateList;

    public static IReadOnlyList<string> ReservedStates => ReservedStateList;

    public static IReadOnlyList<string> BackdropKinds => BackdropKindList;

    /// <summary>该控件是否支持某个状态（批量采集据此跳过无效组合）。</summary>
    public static bool SupportsState(string controlName, string state)
    {
        var canonical = CanonicalName(controlName);
        return state.ToLowerInvariant() switch
        {
            "normal" or "disabled" or "focused" => true,
            "checked" => canonical is "CheckBox" or "ToggleSwitch" or "ToggleButton" or "RadioButton" or "AppBarToggleButton" or "ToggleSplitButton",
            _ => false,
        };
    }

    /// <summary>把大小写不敏感的输入还原成清单里的规范名称；未知名称原样返回。</summary>
    public static string CanonicalName(string name)
    {
        foreach (var known in ControlNameList)
        {
            if (string.Equals(known, name, StringComparison.OrdinalIgnoreCase))
            {
                return known;
            }
        }

        return name;
    }

    /// <summary>该控件在拼版槽位中是否显式指定了固定宽度（供差分器判断固有宽度是否受约束）。</summary>
    public static bool HasExplicitWidth(string controlName)
    {
        return CanonicalName(controlName) is
            "Button" or "Slider" or "ComboBox" or "TextBox" or
            "ToggleButton" or "SplitButton" or "ColorPicker" or "NumberBox" or
            "AutoSuggestBox" or "RichTextBlock" or "ProgressBar" or
            "InfoBar" or "MenuBar" or "CommandBar" or "NavigationView" or
            "TabView" or "Pivot" or "BreadcrumbBar" or "SelectorBar" or "PipsPager" or
            "Expander" or "SplitView" or "Border" or "Card" or "StackPanel" or
            "ScrollView" or "TreeView" or "ListView" or "FlipView" or "CalendarView" or
            "TimePicker" or "TitleBar" or "RichEditBox" or "GridView" or "ListBox" or
            "CommandBarFlyout" or "SwipeControl" or "RefreshContainer" or
            "AnnotatedScrollBar" or "ItemsView" or
            "HyperlinkButton" or "RepeatButton" or "DropDownButton" or "ToggleSplitButton";
    }

    /// <summary>
    /// 计算单个用例的规格指纹（SpecFingerprint）：
    /// 绑定SDK版本、OS版本、DPI缩放、控件规格描述、主题与后景底板配方，供增量缓存判据使用。
    /// </summary>
    public static string ComputeFingerprint(
        string controlName,
        string state,
        string theme,
        string backdrop,
        double rasterizationScale,
        string sdkVersion,
        string osVersion)
    {
        var canonical = CanonicalName(controlName);
        var specSignature = canonical switch
        {
            "Button" => "Button:Content=Button,Width=120,Accent=#005FB8/#60CDFF,Bleed=16",
            "CheckBox" => "CheckBox:Content=CheckBox,AutoSize,Accent=#005FB8/#60CDFF,Bleed=16",
            "ToggleSwitch" => "ToggleSwitch:Header=ToggleSwitch,OnOffEmpty,Accent=#005FB8/#60CDFF,Bleed=16",
            "Slider" => "Slider:Min=0,Max=100,Val=50,Width=200,Accent=#005FB8/#60CDFF,Bleed=16",
            "ComboBox" => "ComboBox:Items=Item 1|Item 2|Item 3,Selected=0,Width=200,Bleed=16",
            "TextBox" => "TextBox:Text=Text,Width=200,Bleed=16",
            _ => canonical + ":DeterministicContent,Themes=light|dark,Bleed=16",
        };

        var raw = string.Join(
            "|",
            CaptureRecipeVersion,
            sdkVersion,
            osVersion,
            rasterizationScale.ToString("F2", System.Globalization.CultureInfo.InvariantCulture),
            canonical,
            state.ToLowerInvariant(),
            theme.ToLowerInvariant(),
            backdrop.ToLowerInvariant(),
            specSignature,
            SystemAccentValue.Value,
            SourceHashValue.Value);

        var hash = SHA256.HashData(Encoding.UTF8.GetBytes(raw));
        return Convert.ToHexString(hash).ToLowerInvariant()[..16];
    }

    private static readonly Lazy<string> SourceHashValue = new(ComputeSourceHash);

    /// <summary>
    /// 对采集宿主目录下的全部 .cs / .xaml 源码计算 SHA-256 前缀。
    /// 宿主二进制或采集逻辑变化时，缓存指纹随之失效，避免继续复用旧二进制产出的参考图。
    /// </summary>
    private static string ComputeSourceHash()
    {
        try
        {
            var directory = FindSourceDirectory(AppContext.BaseDirectory);
            if (directory is null)
            {
                return "unknown";
            }

            var files = Directory
                .EnumerateFiles(directory)
                .Where(path =>
                    path.EndsWith(".cs", StringComparison.OrdinalIgnoreCase) ||
                    path.EndsWith(".xaml", StringComparison.OrdinalIgnoreCase))
                .OrderBy(path => Path.GetFileName(path), StringComparer.Ordinal)
                .ToArray();

            using var buffer = new MemoryStream();
            foreach (var file in files)
            {
                var name = Encoding.UTF8.GetBytes(Path.GetFileName(file));
                buffer.Write(name, 0, name.Length);
                var bytes = File.ReadAllBytes(file);
                buffer.Write(bytes, 0, bytes.Length);
            }

            var hash = SHA256.HashData(buffer.ToArray());
            return Convert.ToHexString(hash).ToLowerInvariant()[..16];
        }
        catch
        {
            return "unknown";
        }
    }

    /// <summary>从输出目录向上查找包含 CaptureWindow.xaml.cs 的采集宿主源码目录。</summary>
    private static string? FindSourceDirectory(string start)
    {
        var directory = new DirectoryInfo(start);
        for (var depth = 0; depth < 10 && directory is not null; depth++)
        {
            if (File.Exists(Path.Combine(directory.FullName, "CaptureWindow.xaml.cs")))
            {
                return directory.FullName;
            }

            directory = directory.Parent;
        }

        return null;
    }

    /// <summary>
    /// 按主题与后景类型生成确定性合成底板画刷（与 unbound-ui 侧 parity::backdrop_color 完全一致）。
    /// </summary>
    public static Brush CreateBackdropBrush(string theme, string backdrop)
    {
        var dark = string.Equals(theme, "dark", StringComparison.OrdinalIgnoreCase);
        var color = (backdrop.ToLowerInvariant(), dark) switch
        {
            // 标准云母合成底板：带轻微冷蓝壁纸色偏的确定性基准色
            ("mica", false) => Color.FromArgb(255, 238, 242, 249),
            ("mica", true) => Color.FromArgb(255, 29, 32, 39),
            // 云母变体（Mica Alt / Tabbed）：层次更深的色调配方
            ("mica_alt", false) => Color.FromArgb(255, 229, 235, 245),
            ("mica_alt", true) => Color.FromArgb(255, 22, 25, 32),
            // 亚克力合成底板：带较高饱和度穿透的确定性底色
            ("acrylic", false) => Color.FromArgb(255, 234, 240, 250),
            ("acrylic", true) => Color.FromArgb(255, 36, 40, 48),
            // 默认纯色基准面（ApplicationPageBackgroundThemeBrush：浅色 #F3F3F3 / 深色 #202020）
            (_, false) => Color.FromArgb(255, 243, 243, 243),
            (_, true) => Color.FromArgb(255, 32, 32, 32),
        };
        return new SolidColorBrush(color);
    }

    public static bool TryCreate(string name, out FrameworkElement element)
    {
        element = null!;

        switch (CanonicalName(name))
        {
            case "Button":
                element = new Button
                {
                    Content = "Button",
                    Width = 120,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "CheckBox":
                element = new CheckBox
                {
                    Content = "CheckBox",
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "ToggleSwitch":
                // Header 即 WinUI 模板的「标题在上、开关在左下」布局。
                // 显式清空 On/OffContent，避免默认本地化文案（中文系统为「开/关」）
                // 混进基线，让比较聚焦于控件几何与颜色。
                element = new ToggleSwitch
                {
                    Header = "ToggleSwitch",
                    OnContent = "",
                    OffContent = "",
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "Slider":
                element = new Slider
                {
                    Minimum = 0,
                    Maximum = 100,
                    Value = 50,
                    Width = 200,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "ComboBox":
                var comboBox = new ComboBox
                {
                    Width = 200,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                comboBox.Items.Add("Item 1");
                comboBox.Items.Add("Item 2");
                comboBox.Items.Add("Item 3");
                // 只截静态控件本体，不展开弹层。
                comboBox.SelectedIndex = 0;
                element = comboBox;
                return true;

            case "TextBox":
                element = new TextBox
                {
                    Text = "Text",
                    Width = 200,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            // ------------------------------------------------------------------
            // 第一批：基础输入与文本类
            // ------------------------------------------------------------------

            case "ToggleButton":
                element = new ToggleButton
                {
                    Content = "ToggleButton",
                    Width = 120,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "SplitButton":
                element = new SplitButton
                {
                    Content = "SplitButton",
                    Width = 120,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "RadioButton":
                element = new RadioButton
                {
                    Content = "RadioButton",
                    GroupName = "ReferenceGroup",
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "RatingControl":
                element = new RatingControl
                {
                    Value = 3,
                    MaxRating = 5,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "ColorPicker":
                element = new ColorPicker
                {
                    Width = 240,
                    // 通道标签（红/绿/蓝）跟随元素 Language 取本地化资源；钉死 en-US
                    // 让基线不随截图机器的系统区域设置变化。
                    Language = "en-US",
                    Color = Colors.DodgerBlue,
                    IsAlphaEnabled = false,
                    IsColorSliderVisible = true,
                    IsColorChannelTextInputVisible = true,
                    IsHexInputVisible = true,
                    IsAlphaSliderVisible = false,
                    IsAlphaTextInputVisible = false,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "NumberBox":
                element = new NumberBox
                {
                    Value = 50,
                    Width = 140,
                    SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Compact,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "AutoSuggestBox":
                element = new AutoSuggestBox
                {
                    Text = "Text",
                    Width = 200,
                    QueryIcon = new SymbolIcon(Symbol.Find),
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "RichTextBlock":
                {
                    var rich = new RichTextBlock
                    {
                        Width = 240,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    var paragraph = new Paragraph();
                    paragraph.Inlines.Add(new Run { Text = "RichTextBlock" });
                    rich.Blocks.Add(paragraph);
                    element = rich;
                    return true;
                }

            case "ProgressBar":
                element = new ProgressBar
                {
                    Value = 40,
                    Width = 200,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "ProgressRing":
                element = new ProgressRing
                {
                    IsActive = true,
                    Width = 32,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "InfoBadge":
                element = new InfoBadge
                {
                    Value = 3,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "ToolTip":
                // ToolTip 是浮层控件；静态采集只测量其内容盒，不主动打开弹层。
                element = new ToolTip { Content = "ToolTip" };
                return true;

            // ------------------------------------------------------------------
            // 第二批：导航、浮层、集合与日期类
            // ------------------------------------------------------------------

            case "InfoBar":
                element = new InfoBar
                {
                    Title = "InfoBar",
                    Message = "Message",
                    Severity = InfoBarSeverity.Informational,
                    IsOpen = true,
                    Width = 320,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "TeachingTip":
                // TeachingTip 弹出层挂在 Popup 根，RenderTargetBitmap 无法直接截取；
                // 使用官方 TeachingTip 主题资源在槽位内构造等值静态呈现表面。
                element = (FrameworkElement)XamlReader.Load(
                    @"<Border xmlns=""http://schemas.microsoft.com/winfx/2006/xaml/presentation""
                             Width=""280"" Height=""96"" CornerRadius=""8"" BorderThickness=""1""
                             Background=""{ThemeResource TeachingTipTransientBackground}""
                             BorderBrush=""{ThemeResource TeachingTipBorderBrush}""
                             Padding=""14,12,14,12"" HorizontalAlignment=""Center"" VerticalAlignment=""Center"">
                         <StackPanel Spacing=""4"">
                             <TextBlock Text=""TeachingTip"" FontSize=""14"" FontWeight=""SemiBold""
                                        Foreground=""{ThemeResource TextFillColorPrimaryBrush}""/>
                             <TextBlock Text=""Message"" FontSize=""12""
                                        Foreground=""{ThemeResource TextFillColorSecondaryBrush}""/>
                         </StackPanel>
                     </Border>");
                return true;

            case "ContentDialog":
                // ContentDialog 未调用 ShowAsync 时处于 Collapsed 状态且 ShowAsync 会打开独立模态 Popup；
                // 使用官方 ContentDialog 主题资源在槽位内构造等值静态呈现表面。
                element = (FrameworkElement)XamlReader.Load(
                    @"<Border xmlns=""http://schemas.microsoft.com/winfx/2006/xaml/presentation""
                             Width=""320"" Height=""180"" CornerRadius=""8"" BorderThickness=""1""
                             Background=""{ThemeResource ContentDialogBackground}""
                             BorderBrush=""{ThemeResource ContentDialogBorderBrush}""
                             HorizontalAlignment=""Center"" VerticalAlignment=""Center"">
                         <Grid>
                             <Grid.RowDefinitions>
                                 <RowDefinition Height=""*""/>
                                 <RowDefinition Height=""Auto""/>
                             </Grid.RowDefinitions>
                             <StackPanel Grid.Row=""0"" Padding=""24,20,24,16"" Spacing=""8"">
                                 <TextBlock Text=""ContentDialog"" FontSize=""20"" FontWeight=""SemiBold""
                                            Foreground=""{ThemeResource TextFillColorPrimaryBrush}""/>
                                 <TextBlock Text=""Message"" FontSize=""14""
                                            Foreground=""{ThemeResource TextFillColorPrimaryBrush}""/>
                             </StackPanel>
                             <Border Grid.Row=""1"" Padding=""24,16,24,16"" BorderThickness=""0,1,0,0""
                                     BorderBrush=""{ThemeResource ContentDialogSeparatorBorderBrush}"">
                                 <Button Content=""OK"" Style=""{StaticResource AccentButtonStyle}""
                                         HorizontalAlignment=""Right"" MinWidth=""120""/>
                             </Border>
                         </Grid>
                     </Border>");
                return true;

            case "Flyout":
                // Flyout 本身不是 FrameworkElement；静态采集以其呈现器为表面。
                element = new FlyoutPresenter
                {
                    Content = new TextBlock { Text = "Flyout" },
                    Width = 200,
                };
                return true;

            case "MenuFlyout":
                {
                    var presenter = new MenuFlyoutPresenter { Width = 200 };
                    presenter.Items.Add(new MenuFlyoutItem { Text = "Item 1" });
                    presenter.Items.Add(new MenuFlyoutItem { Text = "Item 2" });
                    element = presenter;
                    return true;
                }

            case "MenuBar":
                {
                    var bar = new MenuBar { Width = 320 };
                    var fileMenu = new MenuBarItem { Title = "File" };
                    fileMenu.Items.Add(new MenuFlyoutItem { Text = "New" });
                    fileMenu.Items.Add(new MenuFlyoutItem { Text = "Open" });
                    bar.Items.Add(fileMenu);
                    bar.Items.Add(new MenuBarItem { Title = "Edit" });
                    element = bar;
                    return true;
                }

            case "CommandBar":
                {
                    var bar = new CommandBar
                    {
                        Width = 320,
                        DefaultLabelPosition = CommandBarDefaultLabelPosition.Bottom,
                    };
                    bar.PrimaryCommands.Add(new AppBarButton
                    {
                        Label = "Save",
                        Icon = new SymbolIcon(Symbol.Save),
                    });
                    bar.PrimaryCommands.Add(new AppBarButton
                    {
                        Label = "Open",
                        Icon = new SymbolIcon(Symbol.OpenFile),
                    });
                    bar.PrimaryCommands.Add(new AppBarSeparator());
                    element = bar;
                    return true;
                }

            case "NavigationView":
                {
                    var nav = new NavigationView
                    {
                        Width = 300,
                        Height = 200,
                        PaneDisplayMode = NavigationViewPaneDisplayMode.Left,
                        IsSettingsVisible = true,
                        IsBackButtonVisible = NavigationViewBackButtonVisible.Collapsed,
                    };
                    nav.MenuItems.Add(new NavigationViewItem
                    {
                        Content = "Home",
                        Icon = new SymbolIcon(Symbol.Home),
                    });
                    nav.MenuItems.Add(new NavigationViewItem
                    {
                        Content = "Library",
                        Icon = new SymbolIcon(Symbol.Library),
                    });
                    element = nav;
                    return true;
                }

            case "TabView":
                {
                    var tabs = new TabView
                    {
                        Width = 360,
                        Height = 120,
                        IsAddTabButtonVisible = false,
                    };
                    tabs.TabItems.Add(new TabViewItem
                    {
                        Header = "Tab 1",
                        Content = new TextBlock { Text = "Content" },
                    });
                    tabs.TabItems.Add(new TabViewItem { Header = "Tab 2" });
                    tabs.SelectedIndex = 0;
                    element = tabs;
                    return true;
                }

            case "Pivot":
                {
                    var pivot = new Pivot
                    {
                        Width = 320,
                        Height = 120,
                        SelectedIndex = 0,
                    };
                    pivot.Items.Add(new PivotItem
                    {
                        Header = "One",
                        Content = new TextBlock { Text = "One" },
                    });
                    pivot.Items.Add(new PivotItem { Header = "Two" });
                    element = pivot;
                    return true;
                }

            case "BreadcrumbBar":
                element = new BreadcrumbBar
                {
                    Width = 320,
                    ItemsSource = new[] { "Root", "Folder", "File.txt" },
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "SelectorBar":
                {
                    var selector = new SelectorBar { Width = 320 };
                    selector.Items.Add(new SelectorBarItem { Text = "Day" });
                    selector.Items.Add(new SelectorBarItem { Text = "Week" });
                    selector.Items.Add(new SelectorBarItem { Text = "Month" });
                    selector.SelectedItem = selector.Items[0];
                    element = selector;
                    return true;
                }

            case "PipsPager":
                element = new PipsPager
                {
                    NumberOfPages = 5,
                    SelectedPageIndex = 1,
                    MaxVisiblePips = 5,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "Expander":
                element = new Expander
                {
                    Header = "Expander",
                    Content = new TextBlock { Text = "Body" },
                    IsExpanded = true,
                    Width = 320,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "SplitView":
                element = new SplitView
                {
                    Width = 320,
                    Height = 120,
                    DisplayMode = SplitViewDisplayMode.CompactInline,
                    IsPaneOpen = true,
                    OpenPaneLength = 120,
                    Pane = new TextBlock { Text = "Pane" },
                    Content = new TextBlock { Text = "Content" },
                };
                return true;

            case "Border":
                element = (FrameworkElement)XamlReader.Load(
                    @"<Border xmlns=""http://schemas.microsoft.com/winfx/2006/xaml/presentation""
                             Width=""200"" Height=""64"" CornerRadius=""8"" BorderThickness=""1""
                             Background=""{ThemeResource CardBackgroundFillColorDefaultBrush}""
                             BorderBrush=""{ThemeResource CardStrokeColorDefaultBrush}""
                             Padding=""12"" HorizontalAlignment=""Center"" VerticalAlignment=""Center"">
                         <TextBlock Text=""Border"" HorizontalAlignment=""Center"" VerticalAlignment=""Center""/>
                     </Border>");
                return true;

            case "Card":
                element = (FrameworkElement)XamlReader.Load(
                    @"<Border xmlns=""http://schemas.microsoft.com/winfx/2006/xaml/presentation""
                             Width=""240"" Height=""88"" CornerRadius=""8"" BorderThickness=""1""
                             Background=""{ThemeResource CardBackgroundFillColorDefaultBrush}""
                             BorderBrush=""{ThemeResource CardStrokeColorDefaultBrush}""
                             Padding=""12"" HorizontalAlignment=""Center"" VerticalAlignment=""Center"">
                         <TextBlock Text=""Card"" HorizontalAlignment=""Center"" VerticalAlignment=""Center""/>
                     </Border>");
                return true;

            case "StackPanel":
                {
                    var stack = new StackPanel
                    {
                        Width = 200,
                        Spacing = 8,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    stack.Children.Add(new TextBlock { Text = "Row 1" });
                    stack.Children.Add(new TextBlock { Text = "Row 2" });
                    stack.Children.Add(new TextBlock { Text = "Row 3" });
                    element = stack;
                    return true;
                }

            case "ScrollView":
                {
                    var scroll = new ScrollView
                    {
                        Width = 220,
                        Height = 80,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    var inner = new StackPanel { Spacing = 6 };
                    inner.Children.Add(new TextBlock { Text = "Row 1" });
                    inner.Children.Add(new TextBlock { Text = "Row 2" });
                    inner.Children.Add(new TextBlock { Text = "Row 3" });
                    scroll.Content = inner;
                    element = scroll;
                    return true;
                }

            case "TreeView":
                {
                    var tree = new TreeView
                    {
                        Width = 220,
                        Height = 140,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    var root = new TreeViewNode { Content = "Root", IsExpanded = true };
                    root.Children.Add(new TreeViewNode { Content = "Child" });
                    tree.RootNodes.Add(root);
                    tree.SelectedNodes.Add(root);
                    element = tree;
                    return true;
                }

            case "ListView":
                {
                    var list = new ListView
                    {
                        Width = 220,
                        Height = 120,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    list.Items.Add("Item 1");
                    list.Items.Add("Item 2");
                    list.Items.Add("Item 3");
                    list.SelectedIndex = 0;
                    element = list;
                    return true;
                }

            case "FlipView":
                {
                    var flip = new FlipView
                    {
                        Width = 220,
                        Height = 120,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    flip.Items.Add(new TextBlock { Text = "Page 1" });
                    flip.Items.Add(new TextBlock { Text = "Page 2" });
                    element = flip;
                    return true;
                }

            case "CalendarView":
                {
                    var fixedDate = new DateTimeOffset(new DateTime(2026, 1, 15, 0, 0, 0, DateTimeKind.Utc));
                    var cal = new CalendarView
                    {
                        Width = 300,
                        Height = 260,
                        Language = "en-US",
                        CalendarIdentifier = "GregorianCalendar",
                        FirstDayOfWeek = Windows.Globalization.DayOfWeek.Sunday,
                        SelectionMode = CalendarViewSelectionMode.Single,
                        IsTodayHighlighted = false,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    cal.SetDisplayDate(fixedDate);
                    cal.SelectedDates.Add(fixedDate);
                    element = cal;
                    return true;
                }

            case "TimePicker":
                element = new TimePicker
                {
                    Width = 200,
                    Language = "en-US",
                    ClockIdentifier = "24HourClock",
                    SelectedTime = new TimeSpan(13, 30, 0),
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "PersonPicture":
                element = new PersonPicture
                {
                    Initials = "AB",
                    Width = 48,
                    Height = 48,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "Image":
                // 生成确定性 64×64 BGRA 位图作为参考图像，避免空 Source 导致无非背景像素。
                element = new Image
                {
                    Width = 64,
                    Height = 64,
                    Source = CreateDeterministicImageSource(64, 64),
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "FontIcon":
                element = new FontIcon
                {
                    Glyph = "\uE71D",
                    FontSize = 24,
                    Width = 32,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "TitleBar":
                element = new TitleBar
                {
                    Title = "Document",
                    Subtitle = "WinUiReference",
                    Width = 420,
                };
                return true;

            // ------------------------------------------------------------------
            // 第三批：后续补齐的 WinUI 3 控件
            // ------------------------------------------------------------------

            case "RichEditBox":
                {
                    var rich = new RichEditBox
                    {
                        Width = 240,
                        Height = 96,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    rich.Document.SetText(TextSetOptions.None, "RichEditBox");
                    element = rich;
                    return true;
                }

            case "GridView":
                {
                    var grid = new GridView
                    {
                        Width = 260,
                        Height = 140,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    grid.Items.Add("Item 1");
                    grid.Items.Add("Item 2");
                    grid.Items.Add("Item 3");
                    grid.Items.Add("Item 4");
                    grid.SelectedIndex = 0;
                    element = grid;
                    return true;
                }

            case "ListBox":
                {
                    var list = new ListBox
                    {
                        Width = 220,
                        Height = 120,
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    list.Items.Add("Item 1");
                    list.Items.Add("Item 2");
                    list.Items.Add("Item 3");
                    list.SelectedIndex = 0;
                    element = list;
                    return true;
                }

            case "CommandBarFlyout":
                {
                    // CommandBarFlyout 是 FlyoutBase，IsOpen=true 会把内容移入 Popup 根；
                    // 静态采集用带浮层边框的容器包裹闭合态 CommandBar，确保直接落入槽位可视树。
                    var bar = new CommandBar
                    {
                        IsOpen = false,
                        DefaultLabelPosition = CommandBarDefaultLabelPosition.Collapsed,
                        HorizontalAlignment = HorizontalAlignment.Stretch,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    bar.PrimaryCommands.Add(new AppBarButton
                    {
                        Label = "Copy",
                        Icon = new SymbolIcon(Symbol.Copy),
                    });
                    bar.PrimaryCommands.Add(new AppBarButton
                    {
                        Label = "Cut",
                        Icon = new SymbolIcon(Symbol.Cut),
                    });
                    bar.PrimaryCommands.Add(new AppBarSeparator());
                    bar.PrimaryCommands.Add(new AppBarButton
                    {
                        Label = "More",
                        Icon = new SymbolIcon(Symbol.More),
                    });
                    var surface = (Border)XamlReader.Load(
                        @"<Border xmlns=""http://schemas.microsoft.com/winfx/2006/xaml/presentation""
                                 Width=""280"" Height=""48"" CornerRadius=""8"" BorderThickness=""1""
                                 Background=""{ThemeResource FlyoutPresenterBackground}""
                                 BorderBrush=""{ThemeResource FlyoutBorderThemeBrush}""
                                 HorizontalAlignment=""Center"" VerticalAlignment=""Center""/>");
                    surface.Child = bar;
                    element = surface;
                    return true;
                }

            case "AppBarButton":
                element = new AppBarButton
                {
                    Label = "Save",
                    Icon = new SymbolIcon(Symbol.Save),
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "AppBarToggleButton":
                element = new AppBarToggleButton
                {
                    Label = "Bold",
                    Icon = new SymbolIcon(Symbol.Bold),
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "AppBarSeparator":
                element = new AppBarSeparator
                {
                    Height = 40,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "SwipeControl":
                {
                    var swipe = new SwipeControl
                    {
                        Width = 260,
                        Content = (FrameworkElement)XamlReader.Load(
                            @"<Border xmlns=""http://schemas.microsoft.com/winfx/2006/xaml/presentation""
                                     Background=""{ThemeResource CardBackgroundFillColorDefaultBrush}""
                                     Padding=""12"">
                                 <TextBlock Text=""Swipe me""/>
                             </Border>"),
                        HorizontalAlignment = HorizontalAlignment.Center,
                        VerticalAlignment = VerticalAlignment.Center,
                    };
                    var leftItems = new SwipeItems();
                    leftItems.Add(new SwipeItem
                    {
                        Text = "Delete",
                        IconSource = new SymbolIconSource { Symbol = Symbol.Delete },
                        BehaviorOnInvoked = SwipeBehaviorOnInvoked.Close,
                    });
                    swipe.LeftItems = leftItems;
                    element = swipe;
                    return true;
                }

            case "RefreshContainer":
                element = new RefreshContainer
                {
                    Width = 260,
                    Height = 160,
                    Content = new TextBlock { Text = "Pull to refresh" },
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "AnnotatedScrollBar":
                element = new AnnotatedScrollBar
                {
                    Width = 16,
                    Height = 160,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "ItemsView":
                element = new ItemsView
                {
                    Width = 260,
                    Height = 160,
                    ItemsSource = new[] { "Item 1", "Item 2", "Item 3", "Item 4" },
                    SelectionMode = ItemsViewSelectionMode.Single,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            // ------------------------------------------------------------------
            // 第四批：按钮与切换类补充（4）
            // ------------------------------------------------------------------

            case "HyperlinkButton":
                element = new HyperlinkButton
                {
                    Content = "HyperlinkButton",
                    Width = 120,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "RepeatButton":
                element = new RepeatButton
                {
                    Content = "RepeatButton",
                    Width = 120,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "DropDownButton":
                element = new DropDownButton
                {
                    Content = "DropDownButton",
                    Width = 120,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "ToggleSplitButton":
                element = new ToggleSplitButton
                {
                    Content = "ToggleSplitButton",
                    Width = 120,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            // ------------------------------------------------------------------
            // 第五批：输入与日期类补充（3）
            // ------------------------------------------------------------------

            case "PasswordBox":
                element = new PasswordBox
                {
                    Password = "Password",
                    Width = 200,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "DatePicker":
                element = new DatePicker
                {
                    Language = "en-US",
                    CalendarIdentifier = "GregorianCalendar",
                    MonthFormat = "{month.full}",
                    DayFormat = "{day.integer}",
                    YearFormat = "{year.full}",
                    SelectedDate = new DateTimeOffset(new DateTime(2024, 4, 15)),
                    Width = 240,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "CalendarDatePicker":
                element = new CalendarDatePicker
                {
                    Date = new DateTimeOffset(new DateTime(2024, 4, 15)),
                    DateFormat = "{year.full}-{month.integer(2)}-{day.integer(2)}",
                    PlaceholderText = "Select a date",
                    Width = 200,
                    Height = 32,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            default:
                return false;
        }
    }

    /// <summary>
    /// 应用非焦点状态。focused 在这里不做事，必须等 Loaded 之后由 <see cref="ApplyFocus"/> 申请焦点。
    /// </summary>
    public static bool TryApplyState(FrameworkElement element, string state, out string error)
    {
        error = string.Empty;

        switch (state.ToLowerInvariant())
        {
            case "normal":
                return true;

            case "disabled":
                // FrameworkElement 没有 IsEnabled，只有 Control 才有。
                if (element is Control control)
                {
                    control.IsEnabled = false;
                    return true;
                }

                // 纯展示型 FrameworkElement（如 RichTextBlock / FlyoutPresenter）
                // 没有禁用态模板，用统一透明度模拟禁用并禁止命中。
                element.Opacity = 0.4;
                element.IsHitTestVisible = false;
                return true;

            case "checked":
                if (element is CheckBox checkBox)
                {
                    checkBox.IsChecked = true;
                    return true;
                }

                if (element is ToggleSwitch toggleSwitch)
                {
                    toggleSwitch.IsOn = true;
                    return true;
                }

                if (element is ToggleButton toggleButton)
                {
                    toggleButton.IsChecked = true;
                    return true;
                }

                if (element is RadioButton radioButton)
                {
                    radioButton.IsChecked = true;
                    return true;
                }

                if (element is AppBarToggleButton appBarToggle)
                {
                    appBarToggle.IsChecked = true;
                    return true;
                }

                if (element is ToggleSplitButton toggleSplit)
                {
                    toggleSplit.IsChecked = true;
                    return true;
                }

                error = $"Control {element.GetType().Name} does not support checked state";
                return false;

            case "focused":
                return true;

            default:
                error = $"Unknown state: {state}";
                return false;
        }
    }

    public static void ApplyFocus(FrameworkElement element, string state)
    {
        if (string.Equals(state, "focused", StringComparison.OrdinalIgnoreCase))
        {
            // 窗口未激活时可能失败，实际结果记录在用例 JSON 的 controlFocusState 中。
            element.Focus(FocusState.Programmatic);
        }
    }

    /// <summary>生成确定性纯色带 1px 边框的 64×64 BGRA8 位图供 Image 控件像素对齐使用。</summary>
    private static WriteableBitmap CreateDeterministicImageSource(int width, int height)
    {
        var bmp = new WriteableBitmap(width, height);
        var pixels = new byte[width * height * 4];
        for (var y = 0; y < height; y++)
        {
            for (var x = 0; x < width; x++)
            {
                var idx = (y * width + x) * 4;
                var border = x == 0 || y == 0 || x == width - 1 || y == height - 1;
                // 确定性灰蓝占位块（与 unbound-widgets Image 占位图元一致）
                pixels[idx + 0] = border ? (byte)0xB0 : (byte)0xE8; // B
                pixels[idx + 1] = border ? (byte)0x90 : (byte)0xD8; // G
                pixels[idx + 2] = border ? (byte)0x78 : (byte)0xC8; // R
                pixels[idx + 3] = 0xFF;                             // A
            }
        }
        using var stream = bmp.PixelBuffer.AsStream();
        stream.Write(pixels, 0, pixels.Length);
        return bmp;
    }
}
