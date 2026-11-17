using System;
using System.Collections.Generic;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace WinUiReference;

/// <summary>
/// 试点控件清单与构造逻辑。新增控件只需在这里补一条，截图宿主无需改动。
/// 控件内容全部使用确定性文案与固定尺寸，避免系统语言、默认值或窗口宽度影响截图结果。
/// </summary>
internal static class Catalog
{
    private static readonly string[] ControlNameList =
    {
        "Button", "CheckBox", "ToggleSwitch", "Slider", "ComboBox", "TextBox",
    };

    private static readonly string[] ImplementedStateList = { "normal", "disabled", "checked", "focused" };

    // hover / pressed 只保留枚举，尚未实现（需要模拟指针状态，暂不纳入试点）。
    private static readonly string[] ReservedStateList = { "hover", "pressed" };

    public static IReadOnlyList<string> ControlNames => ControlNameList;

    public static IReadOnlyList<string> ImplementedStates => ImplementedStateList;

    public static IReadOnlyList<string> ReservedStates => ReservedStateList;

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
                // 用 Header 而不是 OnContent/OffContent：参考布局是「左侧文案 + 右侧开关」，
                // 与 unbound-ui 的 SwitchDesc 结构一致，避免比较两套不同的排列。
                element = new ToggleSwitch
                {
                    Header = "ToggleSwitch",
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
