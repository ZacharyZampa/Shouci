# Pleco export formats, as observed

Observed from real iOS exports (Pleco 2.0 engine, Oct 2026) in `v1/samples/`, trimmed to a few
cards each. Official docs only specify the text format; XML and `.pqb` are undocumented, so
everything below for them is inferred from the samples.

Sources: Pleco manual, Flashcards chapter
(<https://android.pleco.com/manual/240/flash.html>): text format is specified; "the schema for
the XML flashcard import/export format wasn't finalized yet"; Backup Database saves "cards,
profile settings, scores, categories, etc" to a single file. Forum:
<https://www.plecoforums.com/threads/pleco-2-flashcard-file-format.2287> (`//` category lines).

## 1. Text (`export-text.txt`)

- UTF-8 **with BOM**, **CRLF** line endings.
- First line `// <Category>` (note the space after `//`; the manual shows both).
- Records: `simplified[traditional]<TAB>pinyin`. Export writes the bracket **even when
  identical** (`印象[印象]`). **No definition column** was exported.
- Pinyin uses tone numbers, syllables joined without spaces (`yin4xiang4`); `ü` kept literal
  (`kao3lü4`). `//` inside pinyin marks a split word (`liu2//xia4`, `da3//kai1`) -
  the separable verb-object boundary, not a category line.
- Only one category appears per export; cards in several categories are not repeated.
- Scores, dictionary links and dates are lost.

## 2. XML (`export-flashcards.xml`)

Single line, no whitespace. Root `<plecoflash formatversion="2" creator generator platform created>`
(`created` is Unix seconds). Then `<cards>` of:

```xml
<card language="chinese" created=".." modified="..">
  <entry>
    <headword charset="sc">周围</headword>
    <headword charset="tc">週圍</headword>
    <pron type="hypy" tones="numbers">zhou1wei2</pron>
  </entry>
  <dictref dictid="PACE" entryid="36477440"/>
  <catassign category="Class Words"/>          <!-- repeated, one per category -->
  <scoreinfo scorefile="New Class Words" score="51200" difficulty="120"
     history="666..." correct="17" incorrect="2" reviewed="19" sincelast="0"
     firstreviewedtime=".." lastreviewedtime=".."/>   <!-- 0..n, one per scorefile -->
</card>
```

- Both `sc` and `tc` headwords always present.
- `<defn>` (inside `<entry>`, after `<pron>`, may contain newlines) appears only when the export
  dialog's **Include Data → Dictionary definitions** is on. Our iOS sample had it off.
  See `v1/samples/export-flashcards-android-defn.xml` (reconstructed from the
  [ambuc/pleco-to-anki](https://github.com/ambuc/pleco-to-anki) README, not a verbatim export):
  `platform="Android"`, `pretty-printed`, an empty `<categories/>` before `<cards>`, no
  `created`/`modified` on cards and no `<catassign>`/`<scoreinfo>`. So `categories`, card
  timestamps, category assignments and scores are all optional; parse leniently.
- `dictid` is a 4-char code (`PACE`, `PCED`, `PUNI`) **or** a numeric dictionary creator id
  (`1347367491`); `entryid` is that dictionary's internal key.
- `history` is a string of per-review digits (`6` = perfect, `2`/`3` = fail-ish); the same
  digits appear in the `.pqb`.
- A card may have no `<scoreinfo>` (never reviewed).

## 3. Backup (`backup.pqb`)

A plain **SQLite 3 database** (format string `Pleco SQL Flashcard Database`, FormatVersion 8).

| Table | Content |
| --- | --- |
| `pleco_flash_properties` | file header: FormatString, FormatVersion, FileGenerator, FilePlatform, FileCreated, FileID, FileCreator |
| `pleco_flash_cards` | `id, lang, hw, althw, pron, defn, dictcreator, dictid, dictentry, altdictrefs, wordlength, created, modified` |
| `pleco_flash_categories` | `id, name, created, modified, parent, sort, hidden, class` (top-level parent = `-2`) |
| `pleco_flash_categoryassigns` | `card, cat` pairs |
| `pleco_flash_scorefiles` | `id, name, ...` |
| `pleco_flash_scores_N` | one table per scorefile id N: `card, score, difficulty, history, correct, incorrect, reviewed, sincelastchange, firstreviewedtime, lastreviewedtime, scoreinctime, scoredectime` |
| `pleco_flash_profiles`, `pleco_flash_profilesettings` | test/review profiles and their settings (key/value rows) |
| `pleco_flash_imports` | log of import ranges |

Card encoding quirks:

- `lang` `4096` = Chinese. `hw` simplified, `althw` traditional, `pron` pinyin.
- Words are split per character with `@`: `欢@迎` / `歡@迎` / `huan1@ying2`; pinyin syllables
  within a character group are space-separated: `jin1@tian1 @ji3@hao4`. `wordlength` is the
  character count.
- Dictionary link: `dictcreator`+`dictid`+`dictentry`; `-1/-1/-1` means no link (user-written
  card with `defn` text, e.g. `He ordered steak`).
- Timestamps are Unix seconds. `score`/`difficulty` match the XML values, so XML and `.pqb`
  carry equivalent scheduling data.
- Other `.pqb` files exist: Pleco *user dictionaries* (e.g.
  [alexhk90/Pleco-User-Dictionaries](https://github.com/alexhk90/Pleco-User-Dictionaries)) share the
  extension but are a different format. Detect a flashcard backup via the properties row
  `FormatString = 'Pleco SQL Flashcard Database'`.
- `.pqb` is the only format carrying custom definitions, profiles, and all scorefiles reliably.

## Unverified (do not rely on)

- That re-importing an edited XML preserves scores. The XML has no card id, so matching on
  re-import is unknown (Pleco's "Duplicate Entries" prompt may apply).
- Claims from other sources about `<cardid>`/`<history>` elements or `pleco_flash_history` tables:
  contradicted by the real files above.
