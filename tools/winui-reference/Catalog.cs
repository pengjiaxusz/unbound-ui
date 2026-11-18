using System;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Text;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
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
        "Button", "CheckBox", "ToggleSwitch", "Slider", "ComboBox", "TextBox",
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
            "checked" => canonical is "CheckBox" or "ToggleSwitch",
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
        return CanonicalName(controlName) is "Button" or "Slider" or "ComboBox" or "TextBox";
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
            _ => canonical,
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
