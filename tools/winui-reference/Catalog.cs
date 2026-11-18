using System;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Text;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace WinUiReference;

/// <summary>
/// 试点控件清单、后景底板配方与用例指纹计算。
/// 控件内容全部使用确定性文案与固定尺寸，避免系统语言、默认值或窗口宽度影响截图结果。
/// </summary>
internal static class Catalog
{
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
            "checked" => canonical is "CheckBox" or "ToggleSwitch" or "ToggleButton" or "RadioButton",
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
            "TimePicker" or "TitleBar";
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
            "v2-contact-sheet-roi",
            sdkVersion,
            osVersion,
            rasterizationScale.ToString("F2", System.Globalization.CultureInfo.InvariantCulture),
            canonical,
            state.ToLowerInvariant(),
            theme.ToLowerInvariant(),
            backdrop.ToLowerInvariant(),
            specSignature);

        var hash = SHA256.HashData(Encoding.UTF8.GetBytes(raw));
        return Convert.ToHexString(hash).ToLowerInvariant()[..16];
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
                // IsOpen 需要 XamlRoot；批量宿主在 Loaded 后若无法打开会记录跳过。
                element = new TeachingTip
                {
                    Title = "TeachingTip",
                    Subtitle = "Message",
                    Width = 280,
                };
                return true;

            case "ContentDialog":
                element = new ContentDialog
                {
                    Title = "ContentDialog",
                    Content = "Message",
                    PrimaryButtonText = "OK",
                    Width = 320,
                };
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
                    PaneDisplayMode = SplitViewPaneDisplayMode.Left,
                    IsPaneOpen = true,
                    OpenPaneLength = 120,
                    Pane = new Border
                    {
                        Background = new SolidColorBrush(Color.FromArgb(255, 243, 243, 243)),
                        Child = new TextBlock { Text = "Pane" },
                    },
                    Content = new TextBlock { Text = "Content" },
                };
                return true;

            case "Border":
                element = new Border
                {
                    Width = 200,
                    Height = 64,
                    CornerRadius = new CornerRadius(8),
                    BorderThickness = new Thickness(1),
                    BorderBrush = new SolidColorBrush(Color.FromArgb(255, 224, 224, 224)),
                    Background = new SolidColorBrush(Color.FromArgb(255, 255, 255, 255)),
                    Child = new TextBlock { Text = "Border" },
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "Card":
                element = new Border
                {
                    Width = 240,
                    Height = 88,
                    CornerRadius = new CornerRadius(8),
                    BorderThickness = new Thickness(1),
                    BorderBrush = new SolidColorBrush(Color.FromArgb(255, 224, 224, 224)),
                    Background = new SolidColorBrush(Color.FromArgb(255, 255, 255, 255)),
                    Padding = new Thickness(12),
                    Child = new TextBlock { Text = "Card" },
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
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
                    stack.Children.Add(new TextBlock { Text = "StackPanel" });
                    stack.Children.Add(new TextBlock { Text = "Second row" });
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
                    var root = new TreeViewNode { Content = "Root" };
                    root.Children.Add(new TreeViewNode { Content = "Child" });
                    tree.RootNodes.Add(root);
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
                element = new CalendarView
                {
                    Width = 300,
                    Height = 260,
                    SelectionMode = CalendarViewSelectionMode.Single,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                };
                return true;

            case "TimePicker":
                element = new TimePicker
                {
                    Width = 200,
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
                // 无内置素材，静态采集只测量空图像占位盒。
                element = new Image
                {
                    Width = 64,
                    Height = 64,
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

                error = $"控件 {element.GetType().Name} 不支持 disabled 状态";
                return false;

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

                error = $"控件 {element.GetType().Name} 不支持 checked 状态";
                return false;

            case "focused":
                return true;

            default:
                error = $"未知状态：{state}";
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
}
