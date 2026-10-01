# Pleco UTF-8 text fixtures

`v1/valid/` must parse without errors; `v1/malformed/` must report an error
with a 1-based line number.

## Grammar (`pleco-utf8-text/v1`)

From Pleco's manual (Flashcards → plain text format,
<https://android.pleco.com/manual/240/flash.html#textformat>, same on iOS):

- Records: `characters <tab> pinyin <tab> definition`, UTF-8.
- Characters: simplified, or `simplified[traditional]` when both are given.
- Pinyin and definition may be left out (`characters` alone, or
  `characters <tab><tab> definition`); Pleco fills them in from its
  dictionaries. A supplied definition is always used.
- Pinyin: tone numbers after each syllable (tone marks also accepted).
- Category: a line starting with `//` begins a new category.

Also accepted on import, never written: the Shouci proof of concept's
`[Category]` lines and `%` comment lines.

## Fixtures

| File | Purpose |
| --- | --- |
| `v1/valid/basic-flashcards.txt` | plain characters / pinyin / definition records |
| `v1/valid/categories.txt` | `//` category lines applying to following records |
| `v1/valid/headword-only.txt` | characters only, `simplified[traditional]`, missing pinyin |
| `v1/valid/poc-brackets.txt` | proof-of-concept `[Category]` and `%` comment lines |
| `v1/malformed/empty-headword.txt` | a record with no characters |
