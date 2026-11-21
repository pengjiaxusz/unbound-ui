//! showcase 的命令行解析。
//!
//! 历史上这些入口散落在 `main.rs` 的 `args.iter().any(...)` 探测里，同一个程序既是
//! 展厅又是截图器还是差分器。这里把它们收成四个显式动词：
//!
//! ```text
//! showcase [gallery] [--page <name>]
//! showcase parity-diff [--reference <dir>] [--out <dir>] [--refresh] [--only-failed]
//!                      [--control <a,b>] [--state <a,b>] [--theme <a,b>] [--backdrop <a,b>]
//! showcase shot [<png>] [--dark] [--zoom <125|150|200>] [--width <n>] [--height <n>] [--hud] [--page <name>]
//! showcase backdrop-compare [--name <n>] [--out <dir>] [--material <kind>] [--width <n>] [--height <n>] [--x <n>] [--y <n>]
//! ```
//!
//! 旧旗标（`--parity-run` / `--shot` / `--backdrop-compare`）保留一版兼容，
//! 便于已有脚本与文档平滑迁移；新脚本应使用动词形式。

use crate::png_diff::ParityFilter;

/// 一次进程启动要做的唯一一件事。
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// 打开交互式展厅（默认动作）。
    Gallery(GalleryOptions),
    /// 跑 WinUI 3 参考图的本框架侧离屏差分。
    ParityDiff(ParityDiffOptions),
    /// 离屏渲染整页展厅并写 PNG。
    Shot(ShotOptions),
    /// 原生后景与自绘后景的并排对照。
    BackdropCompare(BackdropCompareOptions),
}

/// 交互式展厅的启动参数。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GalleryOptions {
    /// 启动后直接停留的页面（如 `ParityReport`）。
    pub page: Option<String>,
}

/// 差分参数：路径、缓存开关与用例筛选。
#[derive(Debug, PartialEq, Eq)]
pub struct ParityDiffOptions {
    pub reference: String,
    pub out: String,
    /// 是否复用源码指纹缓存；`--refresh` 置假。
    pub allow_cache: bool,
    /// 只跑上一份报告里未通过的用例。
    pub only_failed: bool,
    pub filter: ParityFilter,
}

impl Default for ParityDiffOptions {
    fn default() -> Self {
        Self {
            reference: "tools/winui-reference/captures".to_string(),
            out: "target/parity".to_string(),
            allow_cache: true,
            only_failed: false,
            filter: ParityFilter::default(),
        }
    }
}

/// 离屏截图参数。
#[derive(Debug, PartialEq, Eq)]
pub struct ShotOptions {
    pub path: Option<String>,
    pub dark: bool,
    /// 缩放档位索引（`ZOOM_STEPS` 的下标）。
    pub zoom_step: usize,
    pub width: u32,
    pub height: u32,
    pub show_hud: bool,
    pub page: Option<String>,
}

impl Default for ShotOptions {
    fn default() -> Self {
        Self {
            path: None,
            dark: false,
            zoom_step: 0,
            width: 1440,
            height: 900,
            show_hud: false,
            page: None,
        }
    }
}

/// 后景对照参数。
#[derive(Debug, PartialEq, Eq)]
pub struct BackdropCompareOptions {
    pub name: String,
    pub out: String,
    pub material: Option<String>,
    pub width: u32,
    pub height: u32,
    pub position: Option<(i32, i32)>,
}

impl Default for BackdropCompareOptions {
    fn default() -> Self {
        Self {
            name: "backdrop".to_string(),
            out: "target/backdrop-compare".to_string(),
            material: None,
            width: 1280,
            height: 800,
            position: None,
        }
    }
}

const USAGE: &str = "usage: showcase [gallery|parity-diff|shot|backdrop-compare] [options]";

/// 解析 `std::env::args()`（含 argv[0]）。
pub fn parse(args: &[String]) -> Result<Command, String> {
    let args = args.get(1..).unwrap_or_default();

    // 旧旗标兼容：历史上用 `--parity-run` / `--shot` / `--backdrop-compare` 选模式。
    if args.iter().any(|arg| arg == "--parity-run") {
        return Ok(Command::ParityDiff(parse_parity_diff(args)?));
    }
    if args.iter().any(|arg| arg == "--shot") {
        return Ok(Command::Shot(parse_shot(args)?));
    }
    if args.iter().any(|arg| arg == "--backdrop-compare") {
        return Ok(Command::BackdropCompare(parse_backdrop_compare(args)?));
    }

    let Some(first) = args.first().map(String::as_str) else {
        return Ok(Command::Gallery(GalleryOptions::default()));
    };

    let rest = &args[1..];
    match first {
        "gallery" => Ok(Command::Gallery(parse_gallery(rest)?)),
        "parity-diff" => Ok(Command::ParityDiff(parse_parity_diff(rest)?)),
        "shot" => Ok(Command::Shot(parse_shot(rest)?)),
        "backdrop-compare" => Ok(Command::BackdropCompare(parse_backdrop_compare(rest)?)),
        other if other.starts_with("--") => {
            // 无动词、只有旗标：按展厅处理，保持 `showcase --page X` 可用。
            Ok(Command::Gallery(parse_gallery(args)?))
        }
        other => Err(format!("unknown command `{other}`\n{USAGE}")),
    }
}

fn parse_gallery(args: &[String]) -> Result<GalleryOptions, String> {
    let mut options = GalleryOptions::default();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--page" {
            options.page = Some(take_value(args, &mut index, "--page")?.to_string());
        } else {
            return Err(format!("unexpected argument `{}`\n{USAGE}", args[index]));
        }
        index += 1;
    }
    Ok(options)
}

fn parse_parity_diff(args: &[String]) -> Result<ParityDiffOptions, String> {
    let mut options = ParityDiffOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--parity-run" => {}
            "--refresh" | "--no-cache" => options.allow_cache = false,
            "--only-failed" => options.only_failed = true,
            "--reference" => {
                options.reference = take_value(args, &mut index, "--reference")?.to_string();
            }
            "--out" => options.out = take_value(args, &mut index, "--out")?.to_string(),
            "--control" => {
                options.filter.controls = split_list(take_value(args, &mut index, "--control")?)
            }
            "--state" => {
                options.filter.states = split_list(take_value(args, &mut index, "--state")?)
            }
            "--theme" => {
                options.filter.themes = split_list(take_value(args, &mut index, "--theme")?)
            }
            "--backdrop" => {
                options.filter.backdrops = split_list(take_value(args, &mut index, "--backdrop")?);
            }
            other => {
                return Err(format!(
                    "unexpected argument `{other}` for parity-diff\n{USAGE}"
                ))
            }
        }
        index += 1;
    }
    Ok(options)
}

fn parse_shot(args: &[String]) -> Result<ShotOptions, String> {
    let mut options = ShotOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--shot" => {
                // 旧语法：`--shot <path>`；新语法用位置参数。
                if let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) {
                    options.path = Some(value.clone());
                    index += 1;
                }
            }
            "--dark" => options.dark = true,
            "--hud" => options.show_hud = true,
            "--zoom" => {
                let percent = take_value(args, &mut index, "--zoom")?;
                options.zoom_step = match percent {
                    "125" => 1,
                    "150" => 2,
                    "200" => 3,
                    _ => 0,
                };
            }
            "--width" => options.width = parse_number(args, &mut index, "--width")?,
            "--height" => options.height = parse_number(args, &mut index, "--height")?,
            "--page" => options.page = Some(take_value(args, &mut index, "--page")?.to_string()),
            other if other.starts_with("--") => {
                return Err(format!("unexpected argument `{other}` for shot\n{USAGE}"));
            }
            path => options.path = Some(path.to_string()),
        }
        index += 1;
    }
    Ok(options)
}

fn parse_backdrop_compare(args: &[String]) -> Result<BackdropCompareOptions, String> {
    let mut options = BackdropCompareOptions::default();
    let mut index = 0;
    let mut x = None;
    let mut y = None;
    while index < args.len() {
        match args[index].as_str() {
            "--backdrop-compare" => {}
            "--name" => options.name = take_value(args, &mut index, "--name")?.to_string(),
            "--out" => options.out = take_value(args, &mut index, "--out")?.to_string(),
            "--material" => {
                options.material = Some(take_value(args, &mut index, "--material")?.to_string());
            }
            "--width" => options.width = parse_number(args, &mut index, "--width")?,
            "--height" => options.height = parse_number(args, &mut index, "--height")?,
            "--x" => x = Some(parse_number(args, &mut index, "--x")?),
            "--y" => y = Some(parse_number(args, &mut index, "--y")?),
            other => {
                return Err(format!(
                    "unexpected argument `{other}` for backdrop-compare\n{USAGE}"
                ));
            }
        }
        index += 1;
    }
    options.position = match (x, y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    };
    Ok(options)
}

fn take_value<'a>(args: &'a [String], index: &mut usize, name: &str) -> Result<&'a str, String> {
    let next = args
        .get(*index + 1)
        .filter(|value| !value.starts_with("--"))
        .map(String::as_str);
    match next {
        Some(value) => {
            *index += 1;
            Ok(value)
        }
        None => Err(format!("{name} requires a value\n{USAGE}")),
    }
}

fn parse_number<T: std::str::FromStr>(
    args: &[String],
    index: &mut usize,
    name: &str,
) -> Result<T, String> {
    let value = take_value(args, index, name)?;
    value
        .parse()
        .map_err(|_| format!("{name} expects a number, got `{value}`\n{USAGE}"))
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, String> {
        let mut full = vec!["showcase".to_string()];
        full.extend(args.iter().map(|arg| (*arg).to_string()));
        parse(&full)
    }

    #[test]
    fn bare_invocation_opens_the_gallery() {
        assert_eq!(
            parse_args(&[]).unwrap(),
            Command::Gallery(GalleryOptions::default())
        );
    }

    #[test]
    fn gallery_accepts_a_starting_page() {
        assert_eq!(
            parse_args(&["gallery", "--page", "ParityReport"]).unwrap(),
            Command::Gallery(GalleryOptions {
                page: Some("ParityReport".to_string())
            })
        );
    }

    #[test]
    fn parity_diff_merges_filters_and_refresh() {
        let Command::ParityDiff(options) = parse_args(&[
            "parity-diff",
            "--control",
            "Button,CheckBox",
            "--theme",
            "dark",
            "--refresh",
            "--only-failed",
        ])
        .unwrap() else {
            panic!("expected a parity-diff command");
        };
        assert!(!options.allow_cache);
        assert!(options.only_failed);
        assert_eq!(options.filter.controls, vec!["Button", "CheckBox"]);
        assert_eq!(options.filter.themes, vec!["dark"]);
        assert_eq!(options.reference, "tools/winui-reference/captures");
    }

    #[test]
    fn legacy_parity_flag_still_runs_the_diff() {
        let command = parse_args(&["--parity-run", "--no-cache", "--out", "target/x"]).unwrap();
        let Command::ParityDiff(options) = command else {
            panic!("expected a parity-diff command");
        };
        assert!(!options.allow_cache);
        assert_eq!(options.out, "target/x");
    }

    #[test]
    fn shot_accepts_positional_path_and_page() {
        let Command::Shot(options) =
            parse_args(&["shot", "target/showcase/x.png", "--dark", "--page", "home"]).unwrap()
        else {
            panic!("expected a shot command");
        };
        assert_eq!(options.path.as_deref(), Some("target/showcase/x.png"));
        assert!(options.dark);
        assert_eq!(options.page.as_deref(), Some("home"));
    }

    #[test]
    fn unknown_verb_is_reported() {
        assert!(parse_args(&["parity"]).is_err());
    }
}
