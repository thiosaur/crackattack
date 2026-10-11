"""Builds the reduced CJK fonts embedded in the game.

Simplified Chinese, Japanese and Korean are drawn with Noto Sans SC, JP and KR, each reduced to
the characters of its own language (a few hundred, instead of 10 to 18 MB per full font). Run
this script after any change to the Chinese, Japanese or Korean texts; the test
`ui_text_has_a_glyph_in_the_embedded_fonts` fails, with this command in its message, when a
character is missing.

    uv run --no-project --with fonttools python tools/fonts/subset_cjk.py          # rebuild
    uv run --no-project --with fonttools python tools/fonts/subset_cjk.py --check  # verify only

Run from the workspace root (rust/). The source fonts are downloaded once into
~/.cache/crackattack-fonts and checked against a pinned SHA-256; they are never committed.
"""

import hashlib
import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATE = ROOT / "crackattack-webgpu"
FONTS_DIR = CRATE / "assets/fonts"
CACHE_DIR = Path.home() / ".cache/crackattack-fonts"

# Google Fonts, pinned to one commit, so every rebuild starts from the same files.
COMMIT = "bd8f81ddb5c74d5c8897b36ad88b440266245103"


class Font:
    """One reduced font: the language whose texts it draws, and its pinned source."""

    def __init__(self, locale, md, variant, family, path, sha256):
        self.locale = locale                  # Locale table in locale.rs (`pub const ZH`)
        self.variant = variant                # `Language::Chinese`: its native name is shown
                                              # in the language selector of every language
        self.md = md                          # text pages of the language (`rules.zh.md`)
        self.output = FONTS_DIR / f"{family}-subset.ttf"
        self.url = f"https://raw.githubusercontent.com/google/fonts/{COMMIT}/ofl/{path}"
        self.sha256 = sha256
        self.cache = CACHE_DIR / f"{family}-{COMMIT[:7]}.ttf"


FONTS = [
    Font("ZH", "zh", "Chinese", "NotoSansSC", "notosanssc/NotoSansSC%5Bwght%5D.ttf",
         "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da"),
    Font("JA", "ja", "Japanese", "NotoSansJP", "notosansjp/NotoSansJP%5Bwght%5D.ttf",
         "c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f"),
    Font("KO", "ko", "Korean", "NotoSansKR", "notosanskr/NotoSansKR%5Bwght%5D.ttf",
         "194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252"),
]

# Characters from U+2E80 up (CJK punctuation, kana, ideographs, Hangul, full-width forms) come
# from the font of the language; everything below is drawn by the default UI font.
CJK_START = 0x2E80


def needed_chars(font):
    """CJK characters of the language: its Locale table, its text pages, and its native name."""
    source = (CRATE / "src/app/locale.rs").read_text()
    table = re.search(r"pub const %s: Locale = Locale \{(.*?)\n\};" % font.locale, source, re.S)
    if table is None:
        sys.exit(f"locale.rs has no `pub const {font.locale}: Locale` table")
    texts = re.findall(r'"((?:[^"\\]|\\.)*)"', table.group(1))
    texts += [p.read_text() for p in sorted((CRATE / "assets/text").glob(f"*.{font.md}.md"))]
    texts += re.findall(r'Language::%s => "([^"]*)"' % font.variant, source)
    return sorted({c for t in texts for c in t if ord(c) >= CJK_START})


def source_font(font):
    if not font.cache.exists():
        font.cache.parent.mkdir(parents=True, exist_ok=True)
        print(f"downloading {font.url}")
        with urllib.request.urlopen(font.url) as r:
            font.cache.write_bytes(r.read())
    digest = hashlib.sha256(font.cache.read_bytes()).hexdigest()
    if digest != font.sha256:
        sys.exit(f"{font.cache}: SHA-256 {digest} differs from the pinned {font.sha256}")
    return font.cache


def covered(path, chars):
    from fontTools.ttLib import TTFont
    cmap = TTFont(path).getBestCmap()
    return [c for c in chars if ord(c) not in cmap]


def build(font, chars):
    from fontTools.ttLib import TTFont
    from fontTools.varLib import instancer
    from fontTools import subset
    tt = TTFont(source_font(font))
    tt = instancer.instantiateVariableFont(tt, {"wght": 400})
    options = subset.Options()
    options.hinting = False
    options.desubroutinize = True
    subsetter = subset.Subsetter(options)
    subsetter.populate(text="".join(chars))
    subsetter.subset(tt)
    tt.recalcTimestamp = False  # keep the source date: a rebuild gives the same bytes
    tt.save(font.output)


def main():
    check = "--check" in sys.argv
    failed = False
    for font in FONTS:
        chars = needed_chars(font)
        if not check:
            build(font, chars)
        missing = covered(font.output, chars) if font.output.exists() else chars
        name = font.output.name
        if missing:
            failed = True
            print(f"{name}: {len(missing)} character(s) missing: {''.join(missing)}")
        else:
            size = font.output.stat().st_size // 1024
            print(f"{name}: covers the {len(chars)} CJK characters of the {font.md} texts ({size} KiB)")
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
