# Pinyin normalization matrix

Spec-first contract for `vocab-pinyin::normalize`, now implemented in Phase 1.
Each row is an input/output pair pinned by unit tests.

Rules, in order:

1. Trim surrounding whitespace.
2. Lowercase.
3. `ü` / `Ü` → `v`; tone-marked `ü` forms carry their tone (`lǚ` → `lv3`).
   CC-CEDICT's `u:` spelling is equivalent (`lu:3` → `lv3`).
4. Tone-marked vowels → plain vowel, tone digit appended at the **end of the
   syllable** (`shuǐ` → `shui3`, `nǐ hǎo` → `ni3 hao3`, `lǚ xíng` → `lv3 xing2`).
   This keeps user input with tone marks equivalent to dictionary data that
   already carries tone numbers (`xíng2` never occurs; it is `xing2`).
5. Apostrophes are preserved unchanged — `xi'an` and `xian` stay distinct.
6. Runs of spaces/tabs collapse to a single space.

`normalize` does not segment or guess tones: `nihao` stays `nihao`. Search
splits unspaced input with `segment()` so `nihao` still finds 你好.

| Input | Expected normalized |
| --- | --- |
| `  NI HAO ` | `ni hao` |
| `nǐ hǎo` | `ni3 hao3` |
| `ni3 hao3` | `ni3 hao3` |
| `shuǐ` | `shui3` |
| `lǚ xíng` | `lv3 xing2` |
| `lu:3 xing2` | `lv3 xing2` |
| `lǜ` | `lv4` |
| `lü` | `lv` |
| `xi'an` | `xi'an` |
| `xian` | `xian` |
| `xǐ'an` | `xi3'an` |
| `ni   hao	zhong` | `ni hao zhong` |
| `  ` | `` |