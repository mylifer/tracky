"""Sürüm notları: önceki etiketten bu yana kullanıcının göreceği commit'ler (feat, fix, perf).

Uygulama ilk açılışta "Yenilikler" olarak gösterir (latest.json'daki `notes`). Kullanım:
`python3 release-notes.py v0.9.75` — etiket yoksa HEAD'e kadar.
"""

import re
import subprocess
import sys

SHOWN = re.compile(r"^(feat|fix|perf)(\([^)]*\))?!?:\s*(.+)$")
# Türkçe büyük harf: i → İ, ı → I.
UPPER = {"i": "İ", "ı": "I"}


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def previous_tag(tag: str) -> str | None:
    tags = [t for t in git("tag", "--list", "v*", "--sort=-v:refname").split() if t != tag]
    target = tag if tag in git("tag", "--list", tag).split() else "HEAD"
    for t in tags:
        # Hedefin atası olan en yeni etiket.
        if subprocess.run(["git", "merge-base", "--is-ancestor", t, target]).returncode == 0:
            return t
    return None


def capitalize(text: str) -> str:
    return UPPER.get(text[0], text[0].upper()) + text[1:] if text else text


def notes(tag: str) -> str:
    target = tag if tag in git("tag", "--list", tag).split() else "HEAD"
    prev = previous_tag(tag)
    span = f"{prev}..{target}" if prev else target
    lines = []
    for subject in git("log", "--no-merges", "--reverse", "--pretty=%s", span).splitlines():
        m = SHOWN.match(subject.strip())
        if m:
            line = f"- {capitalize(m.group(3).strip())}"
            if line not in lines:
                lines.append(line)
    return "\n".join(lines) if lines else "- Küçük düzeltmeler ve iyileştirmeler"


if __name__ == "__main__":
    print(notes(sys.argv[1] if len(sys.argv) > 1 else "HEAD"))
