#!/usr/bin/env python3
"""Validate a two-file external decision pack.

The pack consists of a stable project brief and a per-decision question file.
Documents are authored in Chinese, so the keywords below are Chinese matching
data only. Identifiers, comments and console output stay in English per the
repository convention for skill scripts.

Usage:
    python check_brief.py <project-brief.md> <question.md>

Exit code: 0 when there are no hard failures, 1 otherwise, 2 on bad usage.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

BRIEF_SECTIONS = {
    "position": ("项目定位", "定位与规模", "三十秒读懂"),
    "architecture": ("架构分层", "crate 地图", "crate地图"),
    "dataflow": ("数据流", "一帧"),
    "invariants": ("关键不变量", "隐性约定"),
    "capabilities": ("已实现", "能力清单", "公开能力"),
    "gap": ("组件层", "缺口现状", "不存在的东西"),
    "redlines": ("红线", "不可破"),
    "testing": ("测试", "验收", "工具链"),
    "index": ("文件索引",),
    "discipline": ("事实口径", "答复纪律"),
}

QUESTION_SECTIONS = {
    "prompt": ("可直接复制", "投递语"),
    "references": ("请重点参考", "参考底座"),
    "delta": ("现状增量", "本次特有"),
    "goal": ("目标", "验收"),
    "decisions": ("待决策", "决策问题"),
    "files": ("可能涉及的文件",),
    "contract": ("输出契约", "请按此格式回答"),
    "notice": ("给答复者的提示", "反幻觉"),
}

SESSION_DEPENDENT = ("如上所述", "刚才那份", "我们之前讨论", "前面提到的", "如前所述")
OPEN_ENDED = ("帮我看看", "给点建议", "你怎么看", "给点意见", "深入分析一下")
SPECULATION_WORDS = ("臆测", "编造", "推测")
SENSITIVE_PATTERNS = (
    r"sk-[A-Za-z0-9]{16,}",
    r"ghp_[A-Za-z0-9]{20,}",
    r"-----BEGIN [A-Z ]*PRIVATE KEY",
    r"(?i)password\s*[:=]\s*\S+",
    r"(?i)api[_-]?key\s*[:=]\s*\S+",
)

# Matches references such as "`PROJECT-BRIEF.md` 第 3 节" or
# "PROJECT-BRIEF.md 第 3、5、6 节" inside the question file.
BRIEF_REF = re.compile(
    r"(?:`)?PROJECT-BRIEF\.md(?:`)?\s*第\s*([\d、,，和及\-–\s]+?)\s*节"
)
HEADING_NUMBER = re.compile(r"(?m)^##\s+(\d+)\.")
NUMBER_IN_REF = re.compile(r"\d+")


def split_sections(text: str) -> list[tuple[str, str]]:
    """Split the document on level-two headings into (title, body) pairs."""
    parts = re.split(r"(?m)^##\s+(.+?)\s*$", text)
    sections = []
    for index in range(1, len(parts), 2):
        body = parts[index + 1] if index + 1 < len(parts) else ""
        sections.append((parts[index].strip(), body))
    return sections


def find_body(sections: list[tuple[str, str]], keywords: tuple[str, ...]) -> str | None:
    """Return concatenated bodies for headings matching a keyword group."""
    hits = [body for title, body in sections if any(word in title for word in keywords)]
    return "\n".join(hits) if hits else None


def has_section(sections: list[tuple[str, str]], keywords: tuple[str, ...]) -> bool:
    """Return True when a level-two heading matches one of the keywords."""
    return any(any(word in title for word in keywords) for title, _ in sections)


def quoted_text(body: str) -> str:
    """Extract blockquote lines, skipping authoring hints."""
    lines = []
    for line in body.splitlines():
        stripped = line.strip()
        if not stripped.startswith(">"):
            continue
        content = stripped.lstrip(">").strip()
        if not content or "【填写指引】" in content:
            continue
        lines.append(content)
    return "\n".join(lines)


def numbered_items(body: str) -> int:
    """Count top-level numbered questions inside a section."""
    return len(re.findall(r"(?m)^\s*\d+\.\s+", body))


def brief_references(text: str) -> set[int]:
    """Collect section numbers explicitly attributed to the project brief."""
    found: set[int] = set()
    for match in BRIEF_REF.finditer(text):
        for token in NUMBER_IN_REF.findall(match.group(1)):
            found.add(int(token))
    return found


class Report:
    """Accumulate pass / fail / warning labels with a group prefix."""

    def __init__(self) -> None:
        self.passed: list[str] = []
        self.failed: list[str] = []
        self.warnings: list[str] = []

    def check(self, ok: bool, label: str) -> None:
        (self.passed if ok else self.failed).append(label)

    def warn(self, label: str) -> None:
        self.warnings.append(label)

    def render(self) -> None:
        for label in self.passed:
            print(f"[PASS] {label}")
        for label in self.failed:
            print(f"[FAIL] {label}")
        for label in self.warnings:
            print(f"[WARN] {label}")


def check_hygiene(report: Report, text: str, tag: str) -> None:
    """Shared red lines for both documents."""
    report.check("【填写指引】" not in text, f"{tag}: no authoring hints left")
    found = [word for word in SESSION_DEPENDENT if word in text]
    report.check(not found, f"{tag}: no session-dependent references" + (f": {found}" if found else ""))
    for pattern in SENSITIVE_PATTERNS:
        if re.search(pattern, text):
            report.warn(f"{tag}: possible sensitive data matching /{pattern}/; scrub before sending")
            break
    found_open = [word for word in OPEN_ENDED if word in text]
    if found_open:
        report.warn(f"{tag}: open-ended phrasing found: {found_open}")


def validate_brief(text: str, report: Report) -> set[int]:
    """Validate the stable project brief; return its level-two section numbers."""
    tag = "BRIEF"
    sections = split_sections(text)
    numbers = {int(token) for token in HEADING_NUMBER.findall(text)}

    report.check("【事实】" in text, f"{tag}: verified facts are tagged")
    for key, label in (
        ("position", "project position / scale"),
        ("architecture", "layering / crate map"),
        ("dataflow", "one-frame dataflow"),
        ("invariants", "key invariants"),
        ("capabilities", "implemented capability list"),
        ("gap", "component layer / gap state"),
        ("redlines", "stable red lines"),
        ("testing", "tests / acceptance"),
        ("index", "key file index"),
        ("discipline", "fact policy / answer discipline"),
    ):
        report.check(has_section(sections, BRIEF_SECTIONS[key]), f"{tag}: section: {label}")

    crate_names = set(re.findall(r"\bunbound-[a-z]+\b", text))
    report.check(len(crate_names) >= 10, f"{tag}: crate map covers the workspace (found {len(crate_names)})")

    invariants = find_body(sections, BRIEF_SECTIONS["invariants"]) or ""
    for token, label in (("generation", "generation"), ("排序键", "sort key"),
                         ("颜色表", "color LUT"), ("脏标记", "dirty levels")):
        report.check(token in invariants, f"{tag}: invariants mention {label}")

    policy = find_body(sections, BRIEF_SECTIONS["discipline"]) or ""
    report.check(
        "不确定" in policy and any(word in policy for word in SPECULATION_WORDS),
        f"{tag}: anti-hallucination notice in fact policy",
    )

    decision_headings = [title for title, _ in sections
                         if any(word in title for word in ("待决策", "一句话诉求", "输出契约"))]
    report.check(not decision_headings, f"{tag}: no per-decision sections" + (f": {decision_headings}" if decision_headings else ""))
    report.check("请按以下格式回答" not in text, f"{tag}: no output contract inside the stable brief")

    if "【推断】" not in text and "【未知】" not in text:
        report.warn(f"{tag}: no inference or unknown tags; check facts are not mixed with guesses")
    if len(text) > 20000:
        report.warn(f"{tag}: brief is long ({len(text)} chars); compress rather than dump the codebase")

    check_hygiene(report, text, tag)
    return numbers


def validate_question(text: str, report: Report) -> set[int]:
    """Validate the per-decision question file; return referenced brief sections."""
    tag = "QUESTION"
    sections = split_sections(text)

    report.check("两个文件" in text, f"{tag}: states that two files are submitted")
    refs = brief_references(text)
    report.check("PROJECT-BRIEF.md" in text and len(refs) >= 3,
                 f"{tag}: references project brief sections (found {sorted(refs)})")

    for key, label in (
        ("prompt", "copy-paste submission prompt"),
        ("references", "brief sections to read"),
        ("delta", "decision-specific facts"),
        ("goal", "goal / acceptance criteria"),
        ("decisions", "decisions requested"),
        ("files", "files likely to change"),
        ("contract", "output contract"),
        ("notice", "answerer notice"),
    ):
        report.check(has_section(sections, QUESTION_SECTIONS[key]), f"{tag}: section: {label}")

    report.check(
        all(token in text for token in ("一句话诉求", "决定之后", "怎样算成功")),
        f"{tag}: three decision anchors are stated",
    )

    prompt_body = find_body(sections, QUESTION_SECTIONS["prompt"]) or ""
    prompt_text = quoted_text(prompt_body)
    report.check(len(prompt_text) >= 60, f"{tag}: submission prompt has at least 60 characters")
    if len(prompt_text) > 400:
        report.warn(f"{tag}: submission prompt is long ({len(prompt_text)} chars); keep it under 400")

    questions = find_body(sections, QUESTION_SECTIONS["decisions"]) or ""
    count = numbered_items(questions)
    report.check(count >= 3, f"{tag}: at least 3 numbered decisions (found {count})")
    if count > 8:
        report.warn(f"{tag}: too many decisions ({count}); focus on 3 to 7")
    candidates = len(re.findall(r"候选[:：]", questions))
    report.check(candidates >= 3, f"{tag}: at least 3 decisions offer candidates (found {candidates})")

    report.check("请按以下格式回答" in text, f"{tag}: output contract is quoted verbatim")
    report.check(
        "不确定" in text and any(word in text for word in SPECULATION_WORDS),
        f"{tag}: anti-hallucination notice",
    )
    if len(text) > 8000:
        report.warn(f"{tag}: question file is long ({len(text)} chars); it may be copying the brief")

    check_hygiene(report, text, tag)
    return refs


def main() -> int:
    if len(sys.argv) != 3:
        print("Usage: python check_brief.py <project-brief.md> <question.md>")
        return 2
    paths = [Path(arg) for arg in sys.argv[1:]]
    for path in paths:
        if not path.is_file():
            print(f"[FAIL] file not found: {path}")
            return 1
    texts: list[str] = []
    for path in paths:
        try:
            texts.append(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError) as error:
            print(f"[FAIL] cannot read {path}: {error}")
            return 1

    report = Report()
    brief_numbers = validate_brief(texts[0], report)
    referenced = validate_question(texts[1], report)
    missing = sorted(referenced - brief_numbers)
    report.check(not missing, "PACK: every referenced brief section exists" + (f"; missing {missing}" if missing else ""))

    report.render()
    status = "FAIL" if report.failed else ("WARN" if report.warnings else "PASS")
    print(f"\nResult: {status} ({len(report.failed)} hard failures, {len(report.warnings)} warnings)")
    return 1 if report.failed else 0


if __name__ == "__main__":
    if sys.platform == "win32" and hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.exit(main())
