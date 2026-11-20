#!/usr/bin/env python3
"""Build a self-contained WinUI 3 parity handoff bundle for external review.

Inputs are the products of `just parity-capture --no-cache` and
`just parity --no-cache`: target/parity/report.json, reference/ours/heat PNGs and
the WinUI capture metadata. The output bundle contains side-by-side comparison
images, ROI zoom images, per-case machine-readable diagnostics, a summary index
and a Chinese handoff document rendered from HANDOFF_TEMPLATE.md.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
REPORT = ROOT / "target" / "parity" / "report.json"
PARITY_OUT = ROOT / "target" / "parity"
REFERENCE_DIR = ROOT / "tools" / "winui-reference" / "captures"
TEMPLATE = Path(__file__).resolve().parent / "HANDOFF_TEMPLATE.md"

MEAN_LIMIT = 1.5
P95_LIMIT = 4.0
OVER_LIMIT_MAX = 0.02
BBOX_TOLERANCE = 1
INTRINSIC_TOLERANCE = 1.0

STATUS_ORDER = {"fail": 0, "invalid": 1, "known-aa": 2, "pass": 3}
COLOR_TEXT = (232, 234, 240)
COLOR_MUTED = (150, 158, 176)
COLOR_PANEL = (28, 31, 40)
COLOR_CANVAS = (18, 20, 27)
COLOR_ACCENT = (96, 186, 255)
COLOR_REF = (0, 208, 255)
COLOR_OURS = (255, 176, 48)
COLOR_ROI = (255, 92, 196)


def read_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        json.dump(value, handle, ensure_ascii=False, indent=2)
        handle.write("\n")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_font(size: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    for candidate in (
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\consola.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ):
        try:
            return ImageFont.truetype(candidate, size)
        except OSError:
            continue
    return ImageFont.load_default()


def build_source_map() -> dict[str, dict[str, Any]]:
    """Resolve WinUI canonical names to widget modules and parity emit sites."""
    canonical = (ROOT / "crates/unbound-widgets/src/canonical.rs").read_text(
        encoding="utf-8"
    )
    lib = (ROOT / "crates/unbound-widgets/src/lib.rs").read_text(encoding="utf-8")
    parity = (ROOT / "apps/showcase/src/parity.rs").read_text(encoding="utf-8")

    name_by_variant = dict(
        re.findall(r'Self::(\w+)\s*=>\s*"([^"]+)"', canonical)
    )
    module_by_variant = dict(re.findall(r"^\s*(\w+)\s*=>\s*(\w+)\s*,", lib, re.M))
    primary_match = re.search(
        r"pub const fn is_primary_canonical.*?matches!\((.*?)\)",
        canonical,
        re.S,
    )
    primary = set(re.findall(r"Self::(\w+)", primary_match.group(1))) if primary_match else set()

    module_by_name: dict[str, str] = {}
    for variant, name in name_by_variant.items():
        module = module_by_variant.get(variant)
        if not module:
            continue
        if variant in primary or name not in module_by_name:
            module_by_name[name] = module

    emit_line: dict[str, int] = {}
    start = parity.find("fn emit_case(")
    if start >= 0:
        # start 是字符偏移；必须换算成行号，否则交接文档里的定位会指向几万行之外。
        start_line = parity.count("\n", 0, start) + 1
        tail = parity[start:]
        next_fn = re.search(r"^fn |^pub fn ", tail[1:], re.M)
        body = tail[: next_fn.start() + 1] if next_fn else tail
        for offset, line in enumerate(body.splitlines(), start=1):
            match = re.match(r'\s*"([A-Za-z0-9]+)"\s*=>', line)
            if match:
                emit_line.setdefault(match.group(1), start_line + offset - 1)

    result: dict[str, dict[str, Any]] = {}
    for name, module in module_by_name.items():
        source = ROOT / "crates/unbound-widgets/src" / f"{module}.rs"
        result[name] = {
            "widgetSource": (
                f"crates/unbound-widgets/src/{module}.rs" if source.is_file() else None
            ),
            "emitCaseLine": emit_line.get(name),
            "emitCase": "apps/showcase/src/parity.rs",
            "referenceCatalog": "tools/winui-reference/Catalog.cs",
            "referenceCapture": "tools/winui-reference/CaptureWindow.xaml.cs",
        }
    for name, module in (("Border", "containers"), ("Card", "containers"), ("TitleBar", "title_bar")):
        source = ROOT / "crates/unbound-widgets/src" / f"{module}.rs"
        result[name] = {
            "widgetSource": (
                f"crates/unbound-widgets/src/{module}.rs" if source.is_file() else None
            ),
            "emitCaseLine": emit_line.get(name),
            "emitCase": "apps/showcase/src/parity.rs",
            "referenceCatalog": "tools/winui-reference/Catalog.cs",
            "referenceCapture": "tools/winui-reference/CaptureWindow.xaml.cs",
        }
    return result


def analyze_gates(case: dict[str, Any], reference_meta: dict[str, Any]) -> dict[str, Any]:
    ref_bbox = case["referenceBbox"]
    ours_bbox = case["oursBbox"]
    bbox_delta = [ours - ref for ref, ours in zip(ref_bbox, ours_bbox)]
    bbox_ok = all(abs(delta) <= BBOX_TOLERANCE for delta in bbox_delta)

    intrinsic = case["intrinsic"]
    ref_size = case["referenceSize"]
    explicit_width = bool(reference_meta.get("explicitWidth", False))
    width_ok = explicit_width or abs(intrinsic[0] - ref_size[0]) <= INTRINSIC_TOLERANCE
    height_ok = abs(intrinsic[1] - ref_size[1]) <= INTRINSIC_TOLERANCE
    intrinsic_ok = width_ok and height_ok

    mean_ok = case["meanDe"] <= MEAN_LIMIT
    p95_ok = case["p95De"] <= P95_LIMIT
    over_ok = case["overLimitRatio"] <= OVER_LIMIT_MAX

    failed: list[dict[str, Any]] = []
    if not bbox_ok:
        failed.append(
            {
                "gate": "contentBBox",
                "detail": "ours-reference deltas [left, top, right, bottom] = "
                + json.dumps(bbox_delta),
                "magnitude": max(abs(delta) for delta in bbox_delta),
            }
        )
    if not intrinsic_ok:
        parts = []
        if not width_ok:
            parts.append(f"width {intrinsic[0]:.1f} vs {ref_size[0]:.1f}")
        if not height_ok:
            parts.append(f"height {intrinsic[1]:.1f} vs {ref_size[1]:.1f}")
        failed.append(
            {
                "gate": "intrinsicSize",
                "detail": "; ".join(parts),
                "magnitude": max(
                    abs(intrinsic[0] - ref_size[0]) if not width_ok else 0.0,
                    abs(intrinsic[1] - ref_size[1]) if not height_ok else 0.0,
                ),
            }
        )
    if not mean_ok:
        failed.append(
            {
                "gate": "roiMeanDe",
                "detail": f"{case['meanDe']:.4f} > {MEAN_LIMIT}",
                "magnitude": case["meanDe"] - MEAN_LIMIT,
            }
        )
    if not p95_ok:
        failed.append(
            {
                "gate": "roiP95De",
                "detail": f"{case['p95De']:.4f} > {P95_LIMIT}",
                "magnitude": case["p95De"] - P95_LIMIT,
            }
        )
    if not over_ok:
        failed.append(
            {
                "gate": "significantPixelRatio",
                "detail": f"{case['overLimitRatio'] * 100:.2f}% > {OVER_LIMIT_MAX * 100:.2f}%",
                "magnitude": case["overLimitRatio"] - OVER_LIMIT_MAX,
            }
        )
    failed.sort(key=lambda item: item["magnitude"], reverse=True)

    return {
        "gates": {
            "contentBBox": {
                "ok": bbox_ok,
                "delta": bbox_delta,
                "tolerancePx": BBOX_TOLERANCE,
            },
            "intrinsicSize": {
                "ok": intrinsic_ok,
                "widthOk": width_ok,
                "heightOk": height_ok,
                "explicitWidth": explicit_width,
                "tolerancePx": INTRINSIC_TOLERANCE,
            },
            "roiMeanDe": {
                "ok": mean_ok,
                "value": case["meanDe"],
                "limit": MEAN_LIMIT,
            },
            "roiP95De": {"ok": p95_ok, "value": case["p95De"], "limit": P95_LIMIT},
            "significantPixelRatio": {
                "ok": over_ok,
                "value": case["overLimitRatio"],
                "limit": OVER_LIMIT_MAX,
                "significantDeThreshold": 5.0,
            },
        },
        "failedGates": [item["gate"] for item in failed],
        "failedGateDetails": failed,
        "primaryHint": failed[0]["detail"] if failed else "all gates passed",
    }


def draw_bboxes(
    image: Image.Image,
    case: dict[str, Any],
    show_roi: bool = True,
) -> Image.Image:
    result = image.convert("RGB").copy()
    draw = ImageDraw.Draw(result)

    def outline(box: list[int], color: tuple[int, int, int], width: int = 1) -> None:
        x0, y0, x1, y1 = box
        draw.rectangle([x0, y0, max(x0, x1 - 1), max(y0, y1 - 1)], outline=color, width=width)

    outline(case["referenceBbox"], COLOR_REF)
    outline(case["oursBbox"], COLOR_OURS)
    if show_roi:
        outline(case["roiBbox"], COLOR_ROI)
    return result


def make_panel_grid(
    panels: list[tuple[str, Image.Image]],
    case: dict[str, Any],
    extra_lines: list[str],
) -> Image.Image:
    font = load_font(13)
    small = load_font(11)
    cell_w = max(image.width for _, image in panels) + 16
    cell_h = max(image.height for _, image in panels) + 26
    width = cell_w * len(panels) + 16
    draft = Image.new("RGB", (width, 1), COLOR_CANVAS)
    draft_draw = ImageDraw.Draw(draft)

    def wrap_text(text: str, font_obj: ImageFont.FreeTypeFont | ImageFont.ImageFont) -> list[str]:
        lines: list[str] = []
        current = ""
        for word in text.split(" "):
            candidate = f"{current} {word}".strip()
            if current and draft_draw.textlength(candidate, font=font_obj) > width - 24:
                lines.append(current)
                current = word
            else:
                current = candidate
        lines.append(current)
        return lines

    status = case["status"]
    title = f"{case['name']}  [{status}]  {case['control']} / {case['state']} / {case['theme']} / {case['backdrop']}"
    title_lines = wrap_text(title, font)
    subtitle = "bbox: cyan=reference  orange=ours  magenta=ROI"
    header_h = 12 + 18 * len(title_lines) + 6 + 14 + 10

    wrapped: list[str] = []
    for line in extra_lines:
        wrapped.extend(wrap_text(line, small))
    footer_h = 18 + 17 * len(wrapped)
    height = header_h + cell_h + footer_h + 12
    canvas = Image.new("RGB", (width, height), COLOR_CANVAS)
    draw = ImageDraw.Draw(canvas)

    for line_index, line in enumerate(title_lines):
        draw.text((10, 8 + line_index * 18), line, font=font, fill=COLOR_ACCENT)
    draw.text(
        (10, 12 + 18 * len(title_lines) + 4),
        subtitle,
        font=small,
        fill=COLOR_MUTED,
    )
    draw.line([(8, header_h - 4), (width - 8, header_h - 4)], fill=(55, 60, 74), width=1)

    for index, (label, image) in enumerate(panels):
        x = 8 + index * cell_w
        draw.rounded_rectangle(
            [x + 2, header_h + 2, x + cell_w - 4, header_h + cell_h - 2],
            radius=4,
            fill=COLOR_PANEL,
        )
        draw.text((x + 10, header_h + 5), label, font=small, fill=COLOR_TEXT)
        canvas.paste(image, (x + 8, header_h + 20))

    start = header_h + cell_h + 6
    for line_index, line in enumerate(wrapped):
        draw.text((10, start + line_index * 17), line, font=small, fill=COLOR_MUTED)
    return canvas


def roi_crop(image: Image.Image, roi: list[int]) -> Image.Image:
    width, height = image.size
    x0, y0, x1, y1 = roi
    if x1 <= x0 or y1 <= y0:
        x0, y0, x1, y1 = 0, 0, width, height
    x0 = max(0, min(x0, width - 1))
    y0 = max(0, min(y0, height - 1))
    x1 = max(x0 + 1, min(x1, width))
    y1 = max(y0 + 1, min(y1, height))
    crop = image.crop((x0, y0, x1, y1))
    scale = 2 if max(crop.size) > 180 else 4
    return crop.resize((crop.width * scale, crop.height * scale), Image.Resampling.NEAREST)


def case_lines(case: dict[str, Any], analysis: dict[str, Any]) -> list[str]:
    failed = ", ".join(analysis["failedGates"]) or "-"
    return [
        f"gates failed: {failed} | hint: {analysis['primaryHint']}",
        f"meanDe {case['meanDe']:.3f} (<=1.5) | p95De {case['p95De']:.3f} (<=4.0) | "
        f"over {case['overLimitRatio'] * 100:.2f}% (<=2.00%) | maxDe {case['maxDe']:.3f}",
        f"bbox ref {case['referenceBbox']} ours {case['oursBbox']} roi {case['roiBbox']} | "
        f"intrinsic {case['intrinsic']} vs reference {case['referenceSize']} | "
        f"coverage {case['contentCoverage']:.4f}",
    ]


def markdown_table(headers: list[str], rows: list[list[str]]) -> str:
    head = "| " + " | ".join(headers) + " |"
    separator = "| " + " | ".join("---" for _ in headers) + " |"
    body = ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return "\n".join([head, separator, *body])


def git_head() -> str:
    try:
        return subprocess.check_output(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def build(args: argparse.Namespace) -> int:
    cases = read_json(REPORT)["cases"]
    cases.sort(key=lambda item: (STATUS_ORDER.get(item["status"], 9), item["name"]))
    source_map = build_source_map()
    bundle = args.bundle
    if bundle.exists():
        shutil.rmtree(bundle)
    bundle.mkdir(parents=True)

    counts = {status: 0 for status in STATUS_ORDER}
    gate_histogram: dict[str, int] = {}
    control_stats: dict[str, dict[str, Any]] = {}
    case_index: list[dict[str, Any]] = []
    missing_images: list[str] = []

    for case in cases:
        status = case["status"]
        counts[status] = counts.get(status, 0) + 1
        name = case["name"]
        control = case["control"]
        control_stat = control_stats.setdefault(
            control,
            {
                "control": control,
                "total": 0,
                "pass": 0,
                "known-aa": 0,
                "fail": 0,
                "invalid": 0,
                "failedGates": {},
            },
        )
        control_stat["total"] += 1
        control_stat[status] += 1

        ref_meta_path = REFERENCE_DIR / f"{name}.json"
        ours_meta_path = PARITY_OUT / "ours" / f"{name}.json"
        reference_meta = read_json(ref_meta_path) if ref_meta_path.is_file() else {}
        ours_meta = read_json(ours_meta_path) if ours_meta_path.is_file() else {}
        analysis = analyze_gates(case, reference_meta)
        for gate in analysis["failedGates"]:
            gate_histogram[gate] = gate_histogram.get(gate, 0) + 1
            control_stat["failedGates"][gate] = control_stat["failedGates"].get(gate, 0) + 1

        case_dir = bundle / "cases" / status / name
        case_dir.mkdir(parents=True, exist_ok=True)

        image_specs = {
            "reference": PARITY_OUT / "reference" / f"{name}.png",
            "ours": PARITY_OUT / "ours" / f"{name}.png",
            "heat": PARITY_OUT / "heat" / f"{name}.png",
        }
        images: dict[str, Image.Image] = {}
        for key, path in image_specs.items():
            if path.is_file():
                shutil.copy2(path, case_dir / f"{key}.png")
                images[key] = Image.open(path).convert("RGB")
            else:
                missing_images.append(f"{name}: {key} ({path})")
        for key, path in (("reference", ref_meta_path), ("ours", ours_meta_path)):
            if path.is_file():
                shutil.copy2(path, case_dir / f"{key}.json")

        if images:
            ref = images["reference"]
            ours = images["ours"]
            overlay = Image.blend(ref, ours, 0.5) if ref.size == ours.size else ref.copy()
            panels = [
                ("WinUI 3 reference", draw_bboxes(ref, case)),
                ("Unbound ours", draw_bboxes(ours, case)),
                ("deltaE heat", images.get("heat", ref.copy())),
                ("50/50 overlay", draw_bboxes(overlay, case, show_roi=False)),
            ]
            (case_dir / "compare.png").write_bytes(_image_bytes(make_panel_grid(panels, case, case_lines(case, analysis))))

            zoom_panels = [
                ("reference ROI", roi_crop(ref, case["roiBbox"])),
                ("ours ROI", roi_crop(ours, case["roiBbox"])),
                ("deltaE ROI", roi_crop(images.get("heat", ref), case["roiBbox"])),
            ]
            roi_lines = [
                f"ROI {case['roiBbox']} (union of bboxes + controlBounds, 2px bleed), nearest-neighbour zoom",
                f"reference bbox {case['referenceBbox']} | ours bbox {case['oursBbox']}",
            ]
            (case_dir / "roi_zoom.png").write_bytes(
                _image_bytes(make_panel_grid(zoom_panels, case, roi_lines))
            )

        metrics = {
            "name": name,
            "status": status,
            "report": case,
            "diagnosis": analysis,
            "referenceMetadata": reference_meta,
            "oursMetadata": ours_meta,
            "source": source_map.get(control, {"widgetSource": None, "emitCaseLine": None}),
            "files": {
                "compare": "compare.png" if images else None,
                "roiZoom": "roi_zoom.png" if images else None,
                "reference": "reference.png" if "reference" in images else None,
                "ours": "ours.png" if "ours" in images else None,
                "heat": "heat.png" if "heat" in images else None,
            },
        }
        write_json(case_dir / "metrics.json", metrics)
        case_index.append(
            {
                "name": name,
                "control": control,
                "state": case["state"],
                "theme": case["theme"],
                "status": status,
                "failedGates": analysis["failedGates"],
                "primaryHint": analysis["primaryHint"],
                "meanDe": case["meanDe"],
                "p95De": case["p95De"],
                "overLimitRatio": case["overLimitRatio"],
                "path": f"cases/{status}/{name}/",
            }
        )

    total = len(cases)
    summary = {
        "generatedAtUtc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "gitHead": git_head(),
        "source": {
            "reference": "tools/winui-reference/captures (regenerated with --no-cache --theme all)",
            "diff": "target/parity (regenerated with --no-cache)",
            "report": "target/parity/report.json",
        },
        "counts": counts,
        "total": total,
        "thresholds": {
            "bboxTolerancePx": BBOX_TOLERANCE,
            "intrinsicTolerancePx": INTRINSIC_TOLERANCE,
            "meanDe": MEAN_LIMIT,
            "p95De": P95_LIMIT,
            "overLimitRatio": OVER_LIMIT_MAX,
            "significantDe": 5.0,
        },
        "gateHistogram": gate_histogram,
        "controls": control_stats,
        "cases": case_index,
        "missingImages": missing_images,
    }
    write_json(bundle / "summary.json", summary)
    shutil.copy2(REPORT, bundle / "report.json")

    with (bundle / "summary.csv").open("w", encoding="utf-8-sig", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(
            [
                "name", "control", "state", "theme", "backdrop", "status",
                "failedGates", "primaryHint", "meanDe", "p95De", "p99De",
                "maxDe", "overLimitRatio", "contentCoverage",
                "referenceBbox", "oursBbox", "roiBbox", "intrinsic", "referenceSize",
                "path",
            ]
        )
        for item in case_index:
            case = next(c for c in cases if c["name"] == item["name"])
            writer.writerow(
                [
                    case["name"], case["control"], case["state"], case["theme"],
                    case["backdrop"], case["status"], ";".join(item["failedGates"]),
                    item["primaryHint"], case["meanDe"], case["p95De"], case["p99De"],
                    case["maxDe"], case["overLimitRatio"], case["contentCoverage"],
                    json.dumps(case["referenceBbox"]), json.dumps(case["oursBbox"]),
                    json.dumps(case["roiBbox"]), json.dumps(case["intrinsic"]),
                    json.dumps(case["referenceSize"]), item["path"],
                ]
            )

    fail_cases = [
        json.loads((bundle / item["path"] / "metrics.json").read_text(encoding="utf-8"))
        for item in case_index
        if item["status"] in ("fail", "invalid")
    ]
    write_json(bundle / "actionable_cases.json", fail_cases)

    env_lines = [
        f"generatedAtUtc={summary['generatedAtUtc']}",
        f"gitHead={summary['gitHead']}",
        f"reportSha256={sha256_file(REPORT)}",
        f"caseCount={total}",
        "captureCommand=just parity-capture --no-cache --theme all",
        "diffCommand=just parity --no-cache",
        f"dotnetRoot={os.environ.get('DOTNET_ROOT', '(unset)')}",
        "agentRules=just check / just test / just gate-check",
        f"missingImages={len(missing_images)}",
    ]
    (bundle / "environment.txt").write_text("\n".join(env_lines) + "\n", encoding="utf-8")

    top_controls = sorted(
        control_stats.values(),
        key=lambda item: (-(item["fail"] + item["invalid"]), item["total"]),
    )[:12]
    top_names = {
        stats["control"] for stats in top_controls
    }
    control_rows = []
    for control in sorted(top_names):
        stats = control_stats[control]
        control_rows.append(
            [
                control,
                str(stats["total"]),
                str(stats["fail"] + stats["invalid"]),
                str(stats["pass"]),
                str(stats["known-aa"]),
                ", ".join(
                    f"{gate}×{count}"
                    for gate, count in sorted(
                        stats["failedGates"].items(), key=lambda item: -item[1]
                    )[:3]
                ),
            ]
        )
    gate_rows = [[gate, str(count)] for gate, count in sorted(gate_histogram.items(), key=lambda item: -item[1])]

    actionable_index = [item for item in case_index if item["status"] in ("fail", "invalid")]
    actionable_index.sort(
        key=lambda item: (
            -len(item["failedGates"]),
            -item["overLimitRatio"],
            item["name"],
        )
    )
    top_case_rows = [
        [
            item["name"],
            item["status"],
            ", ".join(item["failedGates"]) or "-",
            f"{item['overLimitRatio'] * 100:.2f}%",
            f"{item['path']}compare.png",
        ]
        for item in actionable_index[:20]
    ]

    template = TEMPLATE.read_text(encoding="utf-8")
    invalid_names = [item["name"] for item in case_index if item["status"] == "invalid"]
    known_names = [item["name"] for item in case_index if item["status"] == "known-aa"]
    handoff = (
        template.replace("{{GENERATED_AT}}", summary["generatedAtUtc"])
        .replace("{{GIT_HEAD}}", summary["gitHead"])
        .replace("{{TOTAL}}", str(total))
        .replace(
            "{{COUNTS}}",
            f"pass {counts['pass']} / known-aa {counts['known-aa']} / fail {counts['fail']} / invalid {counts['invalid']}",
        )
        .replace("{{GATE_TABLE}}", markdown_table(["failed gate", "cases"], gate_rows))
        .replace(
            "{{CONTROL_TABLE}}",
            markdown_table(
                ["control", "total", "fail+invalid", "pass", "known-aa", "top failed gates"],
                control_rows,
            ),
        )
        .replace(
            "{{TOP_CASES_TABLE}}",
            markdown_table(
                ["case", "status", "failed gates", "over %", "comparison"],
                top_case_rows,
            ),
        )
        .replace("{{INVALID_LIST}}", ", ".join(invalid_names) or "none")
        .replace("{{KNOWN_AA_LIST}}", ", ".join(known_names) or "none")
        .replace("{{MISSING_IMAGES}}", str(len(missing_images)))
        .replace("{{ACTIONABLE_COUNT}}", str(len(fail_cases)))
    )
    (bundle / "HANDOFF.md").write_text(handoff, encoding="utf-8", newline="\n")

    args.zip_path.parent.mkdir(parents=True, exist_ok=True)
    if args.zip_path.exists():
        args.zip_path.unlink()
    with zipfile.ZipFile(args.zip_path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for path in sorted(bundle.rglob("*")):
            if path.is_file():
                archive.write(path, Path("parity_bundle") / path.relative_to(bundle))

    print(
        f"bundle={bundle}\nzip={args.zip_path}\ncases={total} counts={counts} "
        f"fail+invalid={len(fail_cases)} missingImages={len(missing_images)}"
    )
    return 0


def _image_bytes(image: Image.Image) -> bytes:
    import io

    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=True)
    return buffer.getvalue()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bundle",
        type=Path,
        default=ROOT / ".scratch" / "parity-handoff" / "bundle",
    )
    parser.add_argument(
        "--zip",
        dest="zip_path",
        type=Path,
        default=ROOT / ".scratch" / "parity-handoff" / "unbound-ui-parity-handoff.zip",
    )
    return build(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())
