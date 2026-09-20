# Pleco UTF-8 text fixtures

`v1/valid/` must parse; `v1/malformed/` must error with a 1-based line number.

## Grammar (`pleco-utf8-text/v1`)

- Records: `headword \t pinyin \t definition` (`\t`-separated, UTF-8).
- Category headers: a line `[Name]` applies to all following records.
- Comments: lines starting with `%` (Pleco convention).
- Blank lines are ignored.
- Record with headword + pinyin but no definition: parsed with a warning (the
  record is preserved, upstream decides).
- Anything else: an error issue with the exact 1-based line number.

## Fixtures

| File | Purpose |
| --- | --- |
| `v1/valid/basic-flashcards.txt` | plain headword / pinyin / gloss records |
| `v1/valid/categories.txt` | `[category]` headers applying to following records |
| `v1/valid/comments.txt` | `%` comment lines, mixed with records |
| `v1/malformed/missing-fields.txt` | missing definition + empty headword lines |