using System.Collections.Generic;
using System.Linq;

namespace WinUiReference;

/// <summary>一个截图用例（四维坐标：控件 x 状态 x 主题 x 后景底板）。</summary>
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

/// <summary>已完成排版计算的单个槽位，包含目标用例与其在画布中的物理坐标与尺寸。</summary>
public sealed record PlannedSlot(CaptureCase Case, double X, double Y, double Width, double Height);

/// <summary>按「控件集 x 主题 x 后景」组织的单张动态拼版页批次。</summary>
public sealed class DynamicParityPage
{
    public DynamicParityPage(string theme, string backdrop, IReadOnlyList<PlannedSlot> slots, int pageIndex = 0)
    {
        Theme = theme;
        Backdrop = backdrop;
        Slots = slots;
        PageIndex = pageIndex;
    }

    public string Theme { get; }

    public string Backdrop { get; }

    public IReadOnlyList<PlannedSlot> Slots { get; }

    /// <summary>本批次内的页号（同主题同底板下的分页序号）。</summary>
    public int PageIndex { get; }

    /// <summary>兼容访问本页包含的所有用例列表。</summary>
    public IReadOnlyList<CaptureCase> Cases => Slots.Select(s => s.Case).ToList();

    /// <summary>本页包含的所有控件规范名列表（去重）。</summary>
    public IReadOnlyList<string> ControlNames =>
        Slots.Select(s => Catalog.CanonicalName(s.Case.ControlName)).Distinct().ToList();
}

/// <summary>按「主题 x 后景」汇聚的候选待测用例批次（待装箱分页）。</summary>
public sealed class CandidateBatch
{
    public CandidateBatch(string theme, string backdrop, IReadOnlyList<CaptureCase> cases)
    {
        Theme = theme;
        Backdrop = backdrop;
        Cases = cases;
    }

    public string Theme { get; }

    public string Backdrop { get; }

    public IReadOnlyList<CaptureCase> Cases { get; }
}
