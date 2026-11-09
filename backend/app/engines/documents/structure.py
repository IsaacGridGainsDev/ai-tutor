import re
from collections import Counter, defaultdict

from app.engines.documents.models import ChapterSpan, Line, SectionSpan, Structure

PAGE_RANGE_SIZE = 20
_NUMBER_WORDS = {w: i for i, w in enumerate(
    "one two three four five six seven eight nine ten eleven twelve".split(), 1)}
_ROMAN = {"i": 1, "v": 5, "x": 10, "l": 50, "c": 100}
_NUM = r"\d{1,3}|[ivxlc]{1,7}|" + "|".join(_NUMBER_WORDS)
_CHAPTER = re.compile(rf"^(chapter|unit|lesson|module)\s+({_NUM})\b[\s:.\-–—]*(.*)$", re.I)
_NUMBERED_TOP = re.compile(r"^(\d{1,2})\.?\s+([A-Z].{2,90})$")
_NUMBERED_SECTION = re.compile(r"^(\d{1,2})\.(\d{1,2})(?:\.\d{1,2})?\.?\s+(\S.{1,100})$")
_DOT_LEADER = re.compile(r"(\.\s?){2,}\s*\d+\s*$")
_TRAILING_PAGE = re.compile(r"\s\d{1,4}\s*$")
_DOTS_ONLY = re.compile(r"^[.\s]+\d*$")
_MIN_CONTENTS_ENTRIES = 5
_MARKER = re.compile(r"^((chapter|part|appendix|section)\s+\w+|[\divxlc.]+)$", re.I)
_LEADING_NUMBER = re.compile(r"^((chapter|part|appendix)\s+\w+[:.]?\s+|[\d.]+\s+)", re.I)


def _is_toc_entry(text: str) -> bool:
    """True for "Some Title ... 12" but not for headings like "Chapter 1" that end in their own number."""
    if _DOT_LEADER.search(text):
        return True
    chapter = _CHAPTER.match(text)
    remainder = chapter.group(3) if chapter else text
    return bool(_TRAILING_PAGE.search(remainder)) and len(re.findall(r"[^\W\d_]{2,}", remainder)) >= 2


def _contents_lines(lines: list[Line], page_count: int) -> set[int]:
    """Lines on table-of-contents pages, which are navigation rather than teachable text."""
    by_page: dict[int, list[int]] = defaultdict(list)
    for index, line in enumerate(lines):
        by_page[line.page].append(index)

    def looks_like_entry(text: str) -> bool:
        if _DOTS_ONLY.match(text) or _is_toc_entry(text):
            return True
        return bool(_TRAILING_PAGE.search(text) and text[:1].isdigit() and re.search(r"[^\W\d_]{2,}", text))

    # Contents live at the front; a limit stops numeric tables later on being mistaken for them.
    front_limit = max(30, page_count * 15 // 100)
    contents_pages: set[int] = set()
    leaders_by_page: dict[int, int] = {}
    for page, indices in by_page.items():
        entries = sum(looks_like_entry(lines[index].text) for index in indices)
        # Some PDFs put number, title, dots and page on separate lines, so dots alone are enough.
        leaders_by_page[page] = sum(bool(_DOTS_ONLY.match(lines[index].text)) for index in indices)
        dense = entries >= _MIN_CONTENTS_ENTRIES and entries >= len(indices) * 0.4
        if page <= front_limit and (dense or leaders_by_page[page] >= _MIN_CONTENTS_ENTRIES):
            contents_pages.add(page)

    # The first and last pages of a contents list can hold only a few entries.
    for page in sorted(contents_pages):
        for neighbour in (page - 1, page + 1):
            if leaders_by_page.get(neighbour):
                contents_pages.add(neighbour)
    return {index for page in contents_pages for index in by_page[page]}


def _norm(text: str) -> str:
    return re.sub(r"[^a-z0-9]", "", text.lower())


def _to_int(token: str) -> int:
    token = token.lower()
    if token.isdigit():
        return int(token)
    if token in _NUMBER_WORDS:
        return _NUMBER_WORDS[token]
    total = 0
    for char, following in zip(token, token[1:] + " "):
        value = _ROMAN[char]
        total += -value if _ROMAN.get(following, 0) > value else value
    return total


def _body_size(lines: list[Line]) -> float:
    weights: Counter[float] = Counter()
    for line in lines:
        weights[round(line.size)] += len(line.text)
    return float(weights.most_common(1)[0][0]) if weights else 10.0


def _is_short_title(text: str) -> bool:
    return len(text) <= 100 and not text.endswith((".", ",", ";", ":", "?"))


def _locate(title: str, indices: list[int], lines: list[Line]) -> tuple[int, set[int]] | None:
    """Find a heading on its page, allowing for wrapped lines and a separate number or label."""
    target = _norm(title)
    targets = {target, _norm(_LEADING_NUMBER.sub("", title))} - {""}
    for pos, _ in enumerate(indices):
        accumulated = ""
        used: list[int] = []
        for index in indices[pos:pos + 4]:
            text = lines[index].text if used else _LEADING_NUMBER.sub("", lines[index].text)
            raw = _norm(lines[index].text)
            if not used and (raw in targets):
                accumulated = raw
                used.append(index)
                break
            accumulated += _norm(text)
            used.append(index)
            if accumulated in targets or not any(t.startswith(accumulated) for t in targets):
                break
        if accumulated in targets:
            first = indices[pos]
            if pos > 0 and _MARKER.match(lines[indices[pos - 1]].text):
                used.insert(0, indices[pos - 1])
                first = indices[pos - 1]
            return first, set(used)
    return None


def _from_outline(toc: list[list], lines: list[Line], page_count: int) -> Structure | None:
    entries = [entry for entry in toc if 1 <= entry[2] <= page_count]
    top_level = [entry for entry in entries if entry[0] == 1]
    level = 2 if len(top_level) == 1 else 1
    if sum(entry[0] == level for entry in entries) < 2:
        return None

    by_page: dict[int, list[int]] = defaultdict(list)
    for index, line in enumerate(lines):
        by_page[line.page].append(index)

    def first_on_or_after(page: int) -> int:
        for candidate in range(page, page_count + 1):
            if by_page[candidate]:
                return by_page[candidate][0]
        return len(lines)

    headings: set[int] = set()
    placed: list[tuple[int, int, str, int]] = []  # (line, level, title, page)
    for entry_level, title, page, *_ in entries:
        if entry_level < level or entry_level > level + 1:
            continue
        found = _locate(title, by_page[page], lines)
        if found:
            start, used = found
            headings |= used
        else:
            start = first_on_or_after(page)
        placed.append((start, entry_level, title.strip(), page))
    placed.sort(key=lambda item: item[0])

    chapters: list[ChapterSpan] = []
    for start, entry_level, title, page in placed:
        if entry_level == level:
            chapters.append(ChapterSpan(title, start, len(lines), page))
            if len(chapters) > 1:
                chapters[-2].end = start
        elif chapters:
            chapters[-1].sections.append(SectionSpan(title, start))
    return Structure("outline", chapters, headings)


def _heading_candidates(lines: list[Line], body: float) -> list[int]:
    return [
        index
        for index, line in enumerate(lines)
        if _is_short_title(line.text)
        and not _is_toc_entry(line.text)
        and (line.size >= body * 1.1 or line.bold)
    ]


def _monotonic(candidates: list[tuple[int, int]]) -> list[tuple[int, int]]:
    kept: list[tuple[int, int]] = []
    for index, number in candidates:
        if not kept or number > kept[-1][1]:
            kept.append((index, number))
    return kept


def _subtitle(index: int, lines: list[Line], body: float) -> tuple[str, list[int]]:
    parts, used = [], []
    for following in range(index + 1, min(index + 4, len(lines))):
        line = lines[following]
        if line.page != lines[index].page or line.size < body * 1.1 or not _is_short_title(line.text):
            break
        parts.append(line.text)
        used.append(following)
    return " ".join(parts), used


def _by_chapter_pattern(lines: list[Line], body: float) -> Structure | None:
    found: list[tuple[int, int]] = []
    for index, line in enumerate(lines):
        match = _CHAPTER.match(line.text)
        if match and len(line.text) <= 100 and not _is_toc_entry(line.text):
            starts_block = index == 0 or lines[index - 1].block != line.block or lines[index - 1].page != line.page
            if line.size >= body * 1.05 or line.bold or starts_block:
                found.append((index, _to_int(match.group(2))))

    # A contents page lists many chapters at once; real chapter openings do not.
    per_page = Counter(lines[index].page for index, _ in found)
    kept = _monotonic([item for item in found if per_page[lines[item[0]].page] < 3])
    if len(kept) < 2:
        return None

    headings: set[int] = set()
    spans: list[tuple[int, str]] = []
    for index, _ in kept:
        match = _CHAPTER.match(lines[index].text)
        label = f"{match.group(1).title()} {match.group(2)}"
        title = match.group(3).strip()
        headings.add(index)
        if not title:
            title, used = _subtitle(index, lines, body)
            headings.update(used)
        spans.append((index, f"{label}: {title}" if title else label))
    return _build("heading_pattern", spans, lines, headings)


def _by_numbered_headings(lines: list[Line], body: float) -> Structure | None:
    found = []
    for index in _heading_candidates(lines, body):
        match = _NUMBERED_TOP.match(lines[index].text)
        if match:
            found.append((index, int(match.group(1))))
    kept: list[tuple[int, int]] = []
    for index, number in found:
        if (not kept and number <= 1) or (kept and number == kept[-1][1] + 1):
            kept.append((index, number))
    if len(kept) < 2:
        return None
    return _build("numbered_heading", [(i, lines[i].text) for i, _ in kept], lines, {i for i, _ in kept})


def _by_font_size(lines: list[Line], body: float, page_count: int) -> Structure | None:
    large = [
        index for index, line in enumerate(lines)
        if line.size >= body * 1.25 and _is_short_title(line.text) and not _is_toc_entry(line.text)
    ]
    tiers: dict[int, list[int]] = defaultdict(list)
    for index in large:
        tiers[round(lines[index].size)].append(index)

    for size in sorted(tiers, reverse=True):
        starts = [
            i for i in tiers[size]
            if i - 1 not in tiers[size] or lines[i - 1].page != lines[i].page
        ]
        if 2 <= len(starts) <= max(2, page_count // 2):
            spans = []
            headings: set[int] = set()
            for start in starts:
                parts, cursor = [], start
                while cursor in tiers[size] and lines[cursor].page == lines[start].page:
                    parts.append(lines[cursor].text)
                    headings.add(cursor)
                    cursor += 1
                spans.append((start, " ".join(parts)))
            return _build("font_size", spans, lines, headings)
    return None


def _by_page_ranges(lines: list[Line], page_count: int) -> Structure:
    if page_count <= PAGE_RANGE_SIZE * 2:
        return Structure("whole_document", [ChapterSpan("Full document", 0, len(lines), 1)], set())
    spans: list[tuple[int, str]] = []
    next_page = 1
    for index, line in enumerate(lines):
        if line.page >= next_page:
            first = line.page
            last = min(first + PAGE_RANGE_SIZE - 1, page_count)
            spans.append((index, f"Pages {first}–{last}"))
            next_page = first + PAGE_RANGE_SIZE
    return _build("page_ranges", spans, lines, set())


def _build(method: str, spans: list[tuple[int, str]], lines: list[Line], headings: set[int]) -> Structure:
    chapters = []
    for position, (start, title) in enumerate(spans):
        end = spans[position + 1][0] if position + 1 < len(spans) else len(lines)
        chapters.append(ChapterSpan(title, start, end, lines[start].page))
    return Structure(method, chapters, headings)


def _detect_sections(chapter: ChapterSpan, lines: list[Line], body: float, headings: set[int]) -> None:
    numbered, styled = [], []
    for index in range(chapter.start, chapter.end):
        line = lines[index]
        if index in headings or _is_toc_entry(line.text):
            continue
        if _NUMBERED_SECTION.match(line.text) and _is_short_title(line.text):
            numbered.append(index)
        elif _is_short_title(line.text) and len(line.text) <= 80 and (line.size >= body * 1.1 or line.bold):
            styled.append(index)
    # Numbered headings are far more reliable, so styled lines are only a fallback.
    chosen = numbered or styled
    chapter.sections = [SectionSpan(lines[index].text, index) for index in chosen]
    headings.update(chosen)


def _add_front_matter(structure: Structure, lines: list[Line]) -> None:
    first = structure.chapters[0].start
    if first > 0:
        structure.chapters.insert(0, ChapterSpan("Front matter", 0, first, lines[0].page))


def detect_structure(lines: list[Line], toc: list[list], page_count: int) -> Structure:
    """Build the chapter and section tree, preferring the most trustworthy signal available."""
    body = _body_size(lines)
    structure = (
        _from_outline(toc, lines, page_count)
        or _by_chapter_pattern(lines, body)
        or _by_numbered_headings(lines, body)
        or _by_font_size(lines, body, page_count)
        or _by_page_ranges(lines, page_count)
    )
    structure.heading_lines |= _contents_lines(lines, page_count)
    _add_front_matter(structure, lines)
    if structure.method not in ("page_ranges", "whole_document"):
        for chapter in structure.chapters:
            if not chapter.sections and chapter.title != "Front matter":
                _detect_sections(chapter, lines, body, structure.heading_lines)

    # Text between a chapter opening and its first section belongs to the chapter itself.
    for chapter in structure.chapters:
        chapter.sections.sort(key=lambda section: section.start)
        if not chapter.sections or chapter.sections[0].start > chapter.start:
            has_lead = any(
                i not in structure.heading_lines
                for i in range(chapter.start, chapter.sections[0].start if chapter.sections else chapter.end)
            )
            if has_lead or not chapter.sections:
                chapter.sections.insert(0, SectionSpan(chapter.title, chapter.start))
    return structure
