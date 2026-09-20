#!/usr/bin/env python3
"""Build fixtures/search/top1000.tsv from HSK 3.0 + dictionary.db."""

from __future__ import annotations

import csv
import re
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HSK = ROOT / "data/dictionary/sources/hsk30-expanded.csv"
OUT = Path(__file__).with_name("top1000.tsv")
MARKS = {
    "ā": "a1",
    "á": "a2",
    "ǎ": "a3",
    "à": "a4",
    "ē": "e1",
    "é": "e2",
    "ě": "e3",
    "è": "e4",
    "ī": "i1",
    "í": "i2",
    "ǐ": "i3",
    "ì": "i4",
    "ō": "o1",
    "ó": "o2",
    "ǒ": "o3",
    "ò": "o4",
    "ū": "u1",
    "ú": "u2",
    "ǔ": "u3",
    "ù": "u4",
    "ǖ": "v1",
    "ǘ": "v2",
    "ǚ": "v3",
    "ǜ": "v4",
    "ü": "v",
}


def dict_path() -> Path:
    installed = Path.home() / "Library/Application Support/pleco-companion/dictionary.db"
    local = ROOT / "data/dictionary/dictionary.db"
    if installed.is_file():
        return installed
    return local


def norm_pinyin(text: str) -> str:
    text = text.strip().lower().replace("ü", "v").replace("u:", "v")
    out: list[str] = []
    pending = None
    for char in text:
        if char in MARKS:
            base_tone = MARKS[char]
            out.append(base_tone[0])
            if len(base_tone) > 1:
                pending = base_tone[1]
        elif char in " \t":
            if pending:
                out.append(pending)
                pending = None
            if out and out[-1] != " ":
                out.append(" ")
        else:
            out.append(char)
    if pending:
        out.append(pending)
    return "".join(out).strip()


def nospace(pinyin: str) -> str:
    return re.sub(r"\s+", "", pinyin.lower().replace("u:", "v"))


def untoned(pinyin: str) -> str:
    return re.sub(r"[1-5]", "", nospace(pinyin))


def core_gloss(gloss: str) -> str:
    gloss = re.sub(r"\s*\(cl:[^)]*\)", "", gloss, flags=re.I)
    gloss = re.split(r"[/(]", gloss, maxsplit=1)[0]
    return " ".join(gloss.split()).strip(" .!;?")


def english_query(gloss: str) -> str | None:
    if gloss[:1].isupper() and not gloss.startswith("CL"):
        return None
    text = core_gloss(gloss).lower()
    if text.startswith(("surname ", "variant of", "used in")):
        return None
    if text.startswith("to "):
        rest = text[3:].strip()
        if 1 <= len(rest.split()) <= 2:
            text = rest
    if not re.fullmatch(r"[a-z][a-z0-9' -]{1,39}", text):
        return None
    if len(text.split()) != 1:
        return None
    if len(text) < 3:
        return None
    stop = {
        "a",
        "an",
        "the",
        "of",
        "to",
        "in",
        "on",
        "for",
        "and",
        "or",
        "is",
        "be",
        "do",
        "i",
        "you",
        "it",
        "at",
        "as",
        "by",
        "we",
        "he",
        "she",
        "my",
        "me",
        "oh",
        "ah",
        "sb",
        "sth",
        # Too many CEDICT senses to reverse-lookup a specific HSK word.
        "close",
        "open",
        "right",
        "left",
        "light",
        "hard",
        "long",
        "short",
        "good",
        "bad",
        "old",
        "new",
        "high",
        "low",
        "big",
        "small",
        "hot",
        "cold",
        "get",
        "make",
        "take",
        "come",
        "look",
        "see",
        "put",
        "set",
        "run",
        "turn",
        "keep",
        "let",
        "call",
        "move",
        "play",
        "start",
        "stop",
        "end",
        "back",
        "down",
        "over",
        "out",
        "off",
        "up",
        "go",
        "come",
    }
    if text in stop:
        return None
    return text


def pick_entry(conn: sqlite3.Connection, simplified: str, hsk_pinyin: str):
    rows = conn.execute(
        "SELECT e.pinyin, g.gloss FROM dictionary_entries e "
        "JOIN dictionary_glosses g ON g.entry_id=e.entry_id AND g.position=1 "
        "WHERE e.simplified=? ORDER BY e.entry_id",
        (simplified,),
    ).fetchall()
    hp_ns = nospace(hsk_pinyin)
    hp_u = untoned(hsk_pinyin)
    exact, neutral, fuzzy = [], [], []
    for pinyin, gloss in rows:
        compact = nospace(pinyin)
        if hp_ns and compact == hp_ns:
            exact.append((pinyin, gloss))
        elif hp_ns and not re.search(r"[1-5]", hp_ns) and compact == hp_ns + "5":
            neutral.append((pinyin, gloss))
        elif untoned(pinyin) == hp_u:
            fuzzy.append((pinyin, gloss))
    for group in (exact, neutral, fuzzy):
        if group:
            return group[0]
    return None


def main() -> None:
    conn = sqlite3.connect(dict_path())
    rows: list[tuple[str, str, str]] = []
    seen_simp: set[str] = set()
    seen_en: set[str] = set()
    with HSK.open(newline="", encoding="utf-8") as handle:
        for rec in csv.DictReader(handle):
            if len(rows) >= 1000:
                break
            simplified = rec["Simplified"].strip()
            if simplified in seen_simp:
                continue
            if not re.fullmatch(r"[\u4e00-\u9fff]+", simplified):
                continue
            chosen = pick_entry(conn, simplified, norm_pinyin(rec["Pinyin"]))
            if not chosen:
                continue
            pinyin, gloss = chosen
            english = english_query(gloss)
            if not english or english in seen_en:
                continue
            compact = nospace(pinyin)
            if re.search(r"[1-5]r|[aeiouv]r", compact):
                continue
            if not re.fullmatch(r"[a-z]{2,}", untoned(pinyin)):
                continue
            seen_simp.add(simplified)
            seen_en.add(english)
            rows.append((simplified, english, pinyin.replace("u:", "v")))
    OUT.write_text(
        "# simplified\tenglish\tpinyin\n"
        + "".join(f"{simp}\t{english}\t{pinyin}\n" for simp, english, pinyin in rows)
    )
    print(f"wrote {len(rows)} probes -> {OUT}")


if __name__ == "__main__":
    main()
