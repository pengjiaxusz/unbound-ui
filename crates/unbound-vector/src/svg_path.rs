//! SVG Path（`M/L/H/V/C/S/Q/T/A/Z`）解析器与二次贝塞尔降阶（S5.6）。

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use unbound_math::Vec2;

use crate::{cubic_to_quads, QuadSegment};

/// SVG 路径 `d` 属性格式错误时返回的错误类型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgPathError {
    /// 语法错误附近的字节偏移量。
    pub offset: usize,
    /// 人类可读的错误原因。
    pub message: &'static str,
}

/// 从 SVG 路径中提取的单条闭合或开放子路径。
#[derive(Clone, Debug, PartialEq)]
pub struct SvgSubpath {
    /// 在 [`SvgPath::segments`] 中的起始索引。
    pub start: usize,
    /// 该子路径包含的 [`QuadSegment`] 数量。
    pub len: usize,
    /// 该子路径是否由 `Z`/`z` 闭合。
    pub closed: bool,
}

/// 已解析并降阶为二次贝塞尔的 SVG 路径。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SvgPath {
    /// [`Self::segments`] 中的子路径切片范围。
    pub subpaths: Vec<SvgSubpath>,
    /// 按顺序排列的所有二次贝塞尔线段。
    pub segments: Vec<QuadSegment>,
    /// 所有控制点的轴对齐 `(min, max)` 包围盒。
    pub bounds: (Vec2, Vec2),
}

impl SvgPath {
    /// 解析 SVG `d` 字符串，并在 `tolerance` 容差内将所有三次曲线/圆弧降阶为二次曲线。
    pub fn parse(d: &str, tolerance: f32) -> Result<Self, SvgPathError> {
        parse_svg_path(d, tolerance)
    }

    /// 在保持宽高比的前提下，将所有线段缩放并平移至目标矩形内。
    #[must_use]
    pub fn scaled_into(
        &self,
        target_origin: Vec2,
        target_size: Vec2,
        viewbox: f32,
    ) -> Vec<QuadSegment> {
        let vb = viewbox.max(1e-3);
        let scale = (target_size.x / vb).min(target_size.y / vb);
        let offset = target_origin
            + Vec2::new(
                (target_size.x - vb * scale) * 0.5,
                (target_size.y - vb * scale) * 0.5,
            );
        self.segments
            .iter()
            .map(|seg| {
                QuadSegment::new(
                    offset + seg.p0 * scale,
                    offset + seg.p1 * scale,
                    offset + seg.p2 * scale,
                )
            })
            .collect()
    }
}

/// 以 `(d_string, quantised_tolerance)` 为键的简易 [`SvgPath`] 解析缓存。
#[derive(Debug, Default)]
pub struct SvgPathCache {
    entries: HashMap<(String, u32), SvgPath>,
}

impl SvgPathCache {
    /// 解析 `d` 或返回此前缓存的 [`SvgPath`]。
    pub fn get_or_parse(&mut self, d: &str, tolerance: f32) -> Result<&SvgPath, SvgPathError> {
        let key = (d.to_owned(), (tolerance.max(1e-4) * 1000.0).round() as u32);
        if let std::collections::hash_map::Entry::Vacant(e) = self.entries.entry(key.clone()) {
            let parsed = parse_svg_path(d, tolerance)?;
            e.insert(parsed);
        }
        Ok(&self.entries[&key])
    }
}

struct Scanner<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            src: s.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws_and_commas(&mut self) {
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b.is_ascii_whitespace() || b == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek_cmd(&mut self) -> Option<u8> {
        self.skip_ws_and_commas();
        let b = *self.src.get(self.pos)?;
        if b.is_ascii_alphabetic() {
            Some(b)
        } else {
            None
        }
    }

    fn next_number(&mut self) -> Result<f32, SvgPathError> {
        self.skip_ws_and_commas();
        let start = self.pos;
        if self.pos >= self.src.len() {
            return Err(SvgPathError {
                offset: start,
                message: "unexpected end of path while reading number",
            });
        }
        if matches!(self.src[self.pos], b'+' | b'-') {
            self.pos += 1;
        }
        let mut seen_digit = false;
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
            self.pos += 1;
            seen_digit = true;
        }
        if self.pos < self.src.len() && self.src[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                self.pos += 1;
                seen_digit = true;
            }
        }
        if !seen_digit {
            return Err(SvgPathError {
                offset: start,
                message: "expected numeric coordinate",
            });
        }
        if self.pos < self.src.len() && matches!(self.src[self.pos], b'e' | b'E') {
            let exp_pos = self.pos;
            self.pos += 1;
            if self.pos < self.src.len() && matches!(self.src[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            let mut exp_digits = false;
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                self.pos += 1;
                exp_digits = true;
            }
            if !exp_digits {
                self.pos = exp_pos;
            }
        }
        let s = core::str::from_utf8(&self.src[start..self.pos]).map_err(|_| SvgPathError {
            offset: start,
            message: "invalid utf-8 in number",
        })?;
        s.parse::<f32>().map_err(|_| SvgPathError {
            offset: start,
            message: "invalid floating-point number",
        })
    }

    fn next_flag(&mut self) -> Result<bool, SvgPathError> {
        self.skip_ws_and_commas();
        let start = self.pos;
        let Some(&b) = self.src.get(self.pos) else {
            return Err(SvgPathError {
                offset: start,
                message: "unexpected end of path while reading arc flag",
            });
        };
        match b {
            b'0' => {
                self.pos += 1;
                Ok(false)
            }
            b'1' => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(SvgPathError {
                offset: start,
                message: "expected 0 or 1 arc flag",
            }),
        }
    }

    fn next_vec2(&mut self) -> Result<Vec2, SvgPathError> {
        let x = self.next_number()?;
        let y = self.next_number()?;
        Ok(Vec2::new(x, y))
    }

    fn has_more_numbers(&mut self) -> bool {
        self.skip_ws_and_commas();
        let Some(&b) = self.src.get(self.pos) else {
            return false;
        };
        b.is_ascii_digit() || b == b'+' || b == b'-' || b == b'.'
    }
}

/// 将 SVG 路径 `d` 属性解析为二次贝塞尔线段。
pub fn parse_svg_path(d: &str, tolerance: f32) -> Result<SvgPath, SvgPathError> {
    let tol = tolerance.max(1e-3);
    let mut scan = Scanner::new(d);
    let mut segments = Vec::new();
    let mut subpaths = Vec::new();

    let mut cur = Vec2::ZERO;
    let mut subpath_start_pt = Vec2::ZERO;
    let mut subpath_seg_start = 0usize;
    let mut last_cubic_ctrl: Option<Vec2> = None;
    let mut last_quad_ctrl: Option<Vec2> = None;
    let mut active_cmd: Option<u8> = None;

    let flush_subpath =
        |subpaths: &mut Vec<SvgSubpath>, seg_start: usize, seg_end: usize, closed: bool| {
            if seg_end > seg_start {
                subpaths.push(SvgSubpath {
                    start: seg_start,
                    len: seg_end - seg_start,
                    closed,
                });
            }
        };

    loop {
        if let Some(cmd) = scan.peek_cmd() {
            scan.pos += 1;
            active_cmd = Some(cmd);
        } else if active_cmd.is_some() && scan.has_more_numbers() {
            // 重复上一条命令（M/m 除外，后续坐标转为 L/l）。
            if active_cmd == Some(b'M') {
                active_cmd = Some(b'L');
            } else if active_cmd == Some(b'm') {
                active_cmd = Some(b'l');
            }
        } else {
            scan.skip_ws_and_commas();
            if scan.pos < scan.src.len() {
                return Err(SvgPathError {
                    offset: scan.pos,
                    message: "unexpected character in svg path",
                });
            }
            break;
        }

        let Some(cmd) = active_cmd else {
            break;
        };
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { cur } else { Vec2::ZERO };

        match cmd {
            b'M' | b'm' => {
                flush_subpath(&mut subpaths, subpath_seg_start, segments.len(), false);
                let pt = base + scan.next_vec2()?;
                cur = pt;
                subpath_start_pt = pt;
                subpath_seg_start = segments.len();
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                while scan.has_more_numbers() {
                    let next_base = if rel { cur } else { Vec2::ZERO };
                    let next = next_base + scan.next_vec2()?;
                    if (next - cur).length_squared() > 1e-10 {
                        segments.push(QuadSegment::line(cur, next));
                    }
                    cur = next;
                }
                active_cmd = Some(if rel { b'l' } else { b'L' });
            }
            b'L' | b'l' => {
                let pt = base + scan.next_vec2()?;
                if (pt - cur).length_squared() > 1e-10 {
                    segments.push(QuadSegment::line(cur, pt));
                }
                cur = pt;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'H' | b'h' => {
                let x = (if rel { cur.x } else { 0.0 }) + scan.next_number()?;
                let pt = Vec2::new(x, cur.y);
                if (pt - cur).length_squared() > 1e-10 {
                    segments.push(QuadSegment::line(cur, pt));
                }
                cur = pt;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'V' | b'v' => {
                let y = (if rel { cur.y } else { 0.0 }) + scan.next_number()?;
                let pt = Vec2::new(cur.x, y);
                if (pt - cur).length_squared() > 1e-10 {
                    segments.push(QuadSegment::line(cur, pt));
                }
                cur = pt;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'C' | b'c' => {
                let c1 = base + scan.next_vec2()?;
                let c2 = base + scan.next_vec2()?;
                let end = base + scan.next_vec2()?;
                segments.extend(cubic_to_quads(cur, c1, c2, end, tol));
                last_cubic_ctrl = Some(c2);
                last_quad_ctrl = None;
                cur = end;
            }
            b'S' | b's' => {
                let c1 = match last_cubic_ctrl {
                    Some(prev) => cur * 2.0 - prev,
                    None => cur,
                };
                let c2 = base + scan.next_vec2()?;
                let end = base + scan.next_vec2()?;
                segments.extend(cubic_to_quads(cur, c1, c2, end, tol));
                last_cubic_ctrl = Some(c2);
                last_quad_ctrl = None;
                cur = end;
            }
            b'Q' | b'q' => {
                let c = base + scan.next_vec2()?;
                let end = base + scan.next_vec2()?;
                segments.push(QuadSegment::new(cur, c, end));
                last_quad_ctrl = Some(c);
                last_cubic_ctrl = None;
                cur = end;
            }
            b'T' | b't' => {
                let c = match last_quad_ctrl {
                    Some(prev) => cur * 2.0 - prev,
                    None => cur,
                };
                let end = base + scan.next_vec2()?;
                segments.push(QuadSegment::new(cur, c, end));
                last_quad_ctrl = Some(c);
                last_cubic_ctrl = None;
                cur = end;
            }
            b'A' | b'a' => {
                let rx = scan.next_number()?.abs();
                let ry = scan.next_number()?.abs();
                let x_axis_rot = scan.next_number()?.to_radians();
                let large_arc = scan.next_flag()?;
                let sweep = scan.next_flag()?;
                let end = base + scan.next_vec2()?;
                arc_to_quads(
                    cur,
                    rx,
                    ry,
                    x_axis_rot,
                    large_arc,
                    sweep,
                    end,
                    tol,
                    &mut segments,
                );
                cur = end;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            b'Z' | b'z' => {
                if (subpath_start_pt - cur).length_squared() > 1e-10 {
                    segments.push(QuadSegment::line(cur, subpath_start_pt));
                }
                cur = subpath_start_pt;
                flush_subpath(&mut subpaths, subpath_seg_start, segments.len(), true);
                subpath_seg_start = segments.len();
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
                active_cmd = None;
            }
            _ => {
                return Err(SvgPathError {
                    offset: scan.pos.saturating_sub(1),
                    message: "unsupported svg path command",
                });
            }
        }
    }

    flush_subpath(&mut subpaths, subpath_seg_start, segments.len(), false);

    let bounds = if let Some(first) = segments.first() {
        let (mut min_b, mut max_b) = first.control_bounds();
        for seg in &segments[1..] {
            let (smin, smax) = seg.control_bounds();
            min_b = min_b.min(smin);
            max_b = max_b.max(smax);
        }
        (min_b, max_b)
    } else {
        (Vec2::ZERO, Vec2::ZERO)
    };

    Ok(SvgPath {
        subpaths,
        segments,
        bounds,
    })
}

/// 将 SVG 椭圆弧转换为三次贝塞尔线段，再降阶为二次贝塞尔线段。
#[allow(clippy::too_many_arguments)]
fn arc_to_quads(
    p0: Vec2,
    mut rx: f32,
    mut ry: f32,
    phi: f32,
    large_arc: bool,
    sweep: bool,
    p1: Vec2,
    tolerance: f32,
    out: &mut Vec<QuadSegment>,
) {
    if (p0 - p1).length_squared() <= 1e-10 {
        return;
    }
    if rx <= 1e-5 || ry <= 1e-5 {
        out.push(QuadSegment::line(p0, p1));
        return;
    }

    let (sin_p, cos_p) = phi.sin_cos();
    let dx2 = (p0.x - p1.x) * 0.5;
    let dy2 = (p0.y - p1.y) * 0.5;
    let x1p = cos_p * dx2 + sin_p * dy2;
    let y1p = -sin_p * dx2 + cos_p * dy2;

    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }

    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let x1p2 = x1p * x1p;
    let y1p2 = y1p * y1p;
    let num = (rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2).max(0.0);
    let den = (rx2 * y1p2 + ry2 * x1p2).max(1e-12);
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let coef = sign * (num / den).sqrt();
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);

    let cx = cos_p * cxp - sin_p * cyp + (p0.x + p1.x) * 0.5;
    let cy = sin_p * cxp + cos_p * cyp + (p0.y + p1.y) * 0.5;

    let vec_angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy))
            .sqrt()
            .max(1e-12);
        let cos_a = (dot / len).clamp(-1.0, 1.0);
        let sign = if ux * vy - uy * vx < 0.0 { -1.0 } else { 1.0 };
        sign * cos_a.acos()
    };

    let vx = (x1p - cxp) / rx;
    let vy = (y1p - cyp) / ry;
    let theta1 = vec_angle(1.0, 0.0, vx, vy);
    let mut dtheta = vec_angle(vx, vy, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if !sweep && dtheta > 0.0 {
        dtheta -= TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += TAU;
    }

    let pieces = ((dtheta.abs() / FRAC_PI_2).ceil() as usize).max(1);
    let step = dtheta / pieces as f32;
    let alpha = (4.0 / 3.0) * (step * 0.25).tan();

    let mut cur = p0;
    let mut a0 = theta1;
    for i in 0..pieces {
        let a1 = a0 + step;
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();

        let ep1 = Vec2::new(c0 - alpha * s0, s0 + alpha * c0);
        let ep2 = Vec2::new(c1 + alpha * s1, s1 - alpha * c1);
        let e_end = Vec2::new(c1, s1);

        let map = |v: Vec2| -> Vec2 {
            let sx = v.x * rx;
            let sy = v.y * ry;
            Vec2::new(cos_p * sx - sin_p * sy + cx, sin_p * sx + cos_p * sy + cy)
        };

        let c_p1 = map(ep1);
        let c_p2 = map(ep2);
        let next = if i + 1 == pieces { p1 } else { map(e_end) };
        out.extend(cubic_to_quads(cur, c_p1, c_p2, next, tolerance));
        cur = next;
        a0 = a1;
    }
    let _ = PI;
}
