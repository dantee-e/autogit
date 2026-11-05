"""Version labelling rules.

Software: semantic versioning (MAJOR.MINOR.PATCH).
CAD: letter revisions (A, B, C...) following ASME Y14.35, which skips I, O, Q, S, X
and Z to avoid confusion with digits. Work-in-progress iterations between formal
revisions are written A.1, A.2...
"""
import re

SEMVER_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
CAD_RE = re.compile(r"^([A-Z]+)(?:\.(\d+))?$")
REV_LETTERS = "ABCDEFGHJKLMNPRTUVWY"

BUMPS = {
    "software": ("patch", "minor", "major"),
    "cad": ("revision", "iteration"),
}
DEFAULT_BUMP = {"software": "minor", "cad": "revision"}

STATUSES = ("draft", "in_review", "approved", "released", "obsolete")
PRIVILEGED_STATUSES = {"approved", "released"}


def _letters_to_int(s: str) -> int:
    n = 0
    for ch in s:
        n = n * len(REV_LETTERS) + REV_LETTERS.index(ch) + 1
    return n


def _int_to_letters(n: int) -> str:
    out = ""
    while n > 0:
        n, r = divmod(n - 1, len(REV_LETTERS))
        out = REV_LETTERS[r] + out
    return out


def next_label(kind: str, previous: str | None, bump: str) -> str:
    if bump not in BUMPS.get(kind, ()):
        raise ValueError(f"Invalid bump '{bump}' for {kind} project")
    if kind == "software":
        major = minor = patch = 0
        if previous:
            m = SEMVER_RE.match(previous)
            if not m:
                raise ValueError(f"Invalid previous label: {previous}")
            major, minor, patch = map(int, m.groups())
        if bump == "major":
            return f"{major + 1}.0.0"
        if bump == "minor":
            return f"{major}.{minor + 1}.0"
        return f"{major}.{minor}.{patch + 1}"

    if not previous:
        return "A" if bump == "revision" else "A.1"
    m = CAD_RE.match(previous)
    if not m or any(c not in REV_LETTERS for c in m.group(1)):
        raise ValueError(f"Invalid previous label: {previous}")
    letters, iteration = m.group(1), int(m.group(2) or 0)
    if bump == "iteration":
        return f"{letters}.{iteration + 1}"
    return _int_to_letters(_letters_to_int(letters) + 1)


def previews(kind: str, previous: str | None) -> dict[str, str]:
    return {b: next_label(kind, previous, b) for b in BUMPS[kind]}
