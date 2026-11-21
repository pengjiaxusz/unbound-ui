using System;
using System.Collections.Generic;
using System.Linq;

namespace WinUiReference;

/// <summary>
/// 命令行参数。所有参数都是可选的；缺省行为等于「全量控件 + 浅色 + solid 底板 + 增量采集」。
///
/// 旗标与 showcase 的差分器保持同名同义：`--refresh` 两侧都表示「不要相信缓存」，
/// 不再出现「同一个名字在两支命令里管两套缓存」的历史陷阱。
/// </summary>
public sealed class CaptureOptions
{
    public const int DefaultWidth = 1920;
    public const int DefaultHeight = 1080;
    public const string DefaultOutputDirectory = "captures";

    private const string Usage =
        "Usage: WinUiReference [--out <directory>] [--control <name1,name2,...>] [--state normal|disabled|checked|focused] " +
        "[--theme light|dark|all] [--backdrop solid|mica|mica_alt|acrylic|all] " +
        "[--review] [--refresh] [--keep-sheets]";

    public string OutputDirectory { get; private init; } = DefaultOutputDirectory;

    /// <summary>显式指定的控件子集；为空表示全部已接入控件。</summary>
    public IReadOnlyList<string> SelectedControls { get; private init; } = Array.Empty<string>();

    /// <summary>用于窗口标题与单值场景的控件名。</summary>
    public string ControlName => SelectedControls.Count > 0 ? SelectedControls[0] : "all";

    public string State { get; private init; } = "normal";

    public string Theme { get; private init; } = "light";

    public string Backdrop { get; private init; } = "solid";

    /// <summary>`--control` 是否由命令行显式给出。</summary>
    public bool ExplicitControl { get; private init; }

    /// <summary>`--state` 是否由命令行显式给出。</summary>
    public bool ExplicitState { get; private init; }

    /// <summary>`--theme` 是否由命令行显式给出。</summary>
    public bool ExplicitTheme { get; private init; }

    /// <summary>人工翻页检视：不采图、不退出，停在页面上等用户翻页。</summary>
    public bool Review { get; private init; }

    /// <summary>忽略指纹缓存，强制重新截图。</summary>
    public bool Refresh { get; private init; }

    /// <summary>除切片外额外落盘未切分的 `_sheet.png` 拼版总览图与坐标索引。</summary>
    public bool KeepSheets { get; private init; }

    /// <summary>展开本次运行要采集的全部候选用例批次（按「后景 x 主题」分组，尚未装箱分页）。</summary>
    public IReadOnlyList<CandidateBatch> BuildCandidateBatches()
    {
        var backdrops = Backdrop == "all"
            ? Catalog.BackdropKinds
            : new[] { Backdrop };

        var themes = Theme == "all"
            ? new[] { "light", "dark" }
            : new[] { Theme };

        IReadOnlyList<string> controls = ExplicitControl ? SelectedControls : Catalog.ControlNames;

        var batches = new List<CandidateBatch>();

        foreach (var backdrop in backdrops)
        {
            foreach (var theme in themes)
            {
                var cases = new List<CaptureCase>();
                foreach (var control in controls)
                {
                    if (ExplicitState)
                    {
                        if (Catalog.SupportsState(control, State))
                        {
                            cases.Add(new CaptureCase(control, State, theme, backdrop));
                        }
                    }
                    else
                    {
                        foreach (var state in Catalog.ImplementedStates)
                        {
                            if (state == "focused" || !Catalog.SupportsState(control, state))
                            {
                                continue;
                            }

                            cases.Add(new CaptureCase(control, state, theme, backdrop));
                        }
                    }
                }

                if (cases.Count > 0)
                {
                    batches.Add(new CandidateBatch(theme, backdrop, cases));
                }
            }
        }

        return batches;
    }

    public static bool TryParse(string[] args, out CaptureOptions options, out string error)
    {
        options = new CaptureOptions();
        error = string.Empty;

        var outputDirectory = DefaultOutputDirectory;
        var selectedControls = new List<string>();
        var state = "normal";
        var theme = "light";
        var backdrop = "solid";
        var review = false;
        var refresh = false;
        var keepSheets = false;
        var explicitControl = false;
        var explicitState = false;
        var explicitTheme = false;

        for (var i = 1; i < args.Length; i++)
        {
            var name = args[i];
            var value = i + 1 < args.Length && !args[i + 1].StartsWith("--", StringComparison.Ordinal)
                ? args[i + 1]
                : null;

            switch (name.ToLowerInvariant())
            {
                case "--review":
                    review = true;
                    break;

                case "--refresh":
                    refresh = true;
                    break;

                // 旧名保留一版：语义已统一为「不信任缓存」，两边同一个词。
                case "--no-cache":
                    Console.Out.WriteLine("warning: --no-cache is deprecated, use --refresh");
                    refresh = true;
                    break;

                case "--keep-sheets":
                    keepSheets = true;
                    break;

                case "--save-sheet":
                    Console.Out.WriteLine("warning: --save-sheet is deprecated, use --keep-sheets");
                    keepSheets = true;
                    break;

                case "--batch":
                    error = UsageError(
                        "--batch is gone: capturing every control is now the default, use --control to narrow the set");
                    return false;

                case "--width":
                case "--height":
                    error = UsageError(
                        $"{name} is gone: the capture canvas is pinned to 1920x1080 DIP so slices stay comparable");
                    return false;

                case "--out":
                    if (value is null)
                    {
                        error = UsageError("--out missing directory argument");
                        return false;
                    }

                    outputDirectory = value;
                    i++;
                    break;

                case "--control":
                    if (value is null)
                    {
                        error = UsageError("--control missing control name");
                        return false;
                    }

                    var names = value.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
                    if (names.Length == 0)
                    {
                        error = UsageError("--control missing valid control name");
                        return false;
                    }

                    var validated = new List<string>();
                    foreach (var raw in names)
                    {
                        if (!Catalog.ControlNames.Contains(raw, StringComparer.OrdinalIgnoreCase))
                        {
                            error = UsageError($"Unknown control: {raw}{Environment.NewLine}Available controls: {string.Join(", ", Catalog.ControlNames)}");
                            return false;
                        }
                        validated.Add(Catalog.CanonicalName(raw));
                    }

                    selectedControls = validated;
                    explicitControl = true;
                    i++;
                    break;

                case "--state":
                    if (value is null)
                    {
                        error = UsageError("--state missing state name");
                        return false;
                    }

                    state = value.ToLowerInvariant();
                    explicitState = true;
                    i++;
                    break;

                case "--theme":
                    if (value is null)
                    {
                        error = UsageError("--theme missing theme name");
                        return false;
                    }

                    theme = value.ToLowerInvariant();
                    explicitTheme = true;
                    i++;
                    break;

                case "--backdrop":
                    if (value is null)
                    {
                        error = UsageError("--backdrop missing backdrop type");
                        return false;
                    }

                    backdrop = value.ToLowerInvariant();
                    i++;
                    break;

                default:
                    error = UsageError($"Unknown argument: {name}");
                    return false;
            }
        }

        if (backdrop != "all" && !Catalog.BackdropKinds.Contains(backdrop, StringComparer.OrdinalIgnoreCase))
        {
            error = UsageError($"Unknown backdrop type: {backdrop}, available: {string.Join(", ", Catalog.BackdropKinds)}, all");
            return false;
        }

        if (explicitState)
        {
            if (!Catalog.ImplementedStates.Contains(state, StringComparer.OrdinalIgnoreCase))
            {
                error = Catalog.ReservedStates.Contains(state, StringComparer.OrdinalIgnoreCase)
                    ? UsageError($"State {state} is not implemented yet (enum reserved), use {string.Join("/", Catalog.ImplementedStates)}")
                    : UsageError($"Unknown state: {state}, available states: {string.Join(", ", Catalog.ImplementedStates)}");
                return false;
            }
        }

        if (theme != "light" && theme != "dark" && theme != "all")
        {
            error = UsageError($"Unknown theme: {theme}, available themes: light, dark, all");
            return false;
        }

        if (string.IsNullOrWhiteSpace(outputDirectory))
        {
            error = UsageError("--out cannot be empty");
            return false;
        }

        options = new CaptureOptions
        {
            OutputDirectory = outputDirectory,
            SelectedControls = selectedControls,
            State = state,
            Theme = theme,
            Backdrop = backdrop,
            Review = review,
            Refresh = refresh,
            KeepSheets = keepSheets,
            ExplicitControl = explicitControl,
            ExplicitState = explicitState,
            ExplicitTheme = explicitTheme,
        };

        return true;
    }

    private static string UsageError(string message)
    {
        return $"{message}{Environment.NewLine}{Usage}";
    }
}
