#!/usr/bin/env python3
"""Enforce the sprint-status.yaml invariants that CI must keep honest.

The file header documents the key lifecycle (backlog -> in-progress -> review -> done) and
why a stale key is a wrong answer rather than a missing one: the status file is the only
place sprint progress is legible. This check makes that legibility a build invariant,
closing the epic-3 retrospective's third-recurrence finding — work that shipped was left
at `review` (twice before, once more in epic 3), so the file reported outstanding work
that had shipped and could not close its epic from its own record.

It fails when:
  * an epic key is `done` while any of its story keys sits below `done` (the recurring
    lag: the epic was closed from a file that lied); or
  * every story key of an epic is terminal (`done`/`skipped`) while the epic key is not
    `done` (the mirror image: fully accepted work, an epic the file still says is open).

It also fails closed on format drift: a status value outside the documented lifecycle,
or a development_status line the minimal parser cannot read, is an error, not a skip.

Stdlib only — CI runners get python3 on every image, and the file's flat `key: value`
shape under `development_status:` needs nothing else.

Usage: check_sprint_status.py [path/to/sprint-status.yaml]
Exit: 0 clean, 1 invariant or format violation, 2 usage error.
"""

import re
import sys

LIFECYCLE = {"backlog", "in-progress", "review", "done", "skipped"}
TERMINAL = {"done", "skipped"}

# Story keys are `<epic>-<n>-slug` (numeric epic); epic keys are `epic-<n>`; retrospective
# keys are `epic-<n>-retrospective` (tracked, but not a story).
STORY_KEY = re.compile(r"^(\d+)-\d+-")
EPIC_KEY = re.compile(r"^epic-(\d+)$")
RETRO_KEY = re.compile(r"^epic-\d+-retrospective$")

# Retrospective keys of not-yet-reached epics are pre-created as `optional` by the planning
# skills; that is a planning placeholder, not a lifecycle state, and gets its own allowance.
RETRO_PLACEHOLDER = {"optional"} | TERMINAL


def parse_development_status(text):
    """Returns (entries, problems): ordered [(key, value)] plus format errors."""
    entries = []
    problems = []
    in_section = False
    for lineno, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line.startswith(" "):
            in_section = line.rstrip() == "development_status:"
            continue
        if not in_section:
            continue
        if line.startswith("    "):
            problems.append(
                f"line {lineno}: nested entry under development_status is not part of the "
                f"flat key: value shape this parser reads; refusing to guess: {line.strip()}"
            )
            continue
        match = re.match(r"^  ([A-Za-z0-9_][A-Za-z0-9_-]*):[ \t]+([A-Za-z0-9_-]+)", line)
        if not match:
            problems.append(
                f"line {lineno}: cannot parse development_status entry: {line.strip()}"
            )
            continue
        key, value = match.group(1), match.group(2)
        entries.append((key, value))
        allowed = RETRO_PLACEHOLDER if RETRO_KEY.match(key) else LIFECYCLE
        if value not in allowed:
            problems.append(
                f"line {lineno}: key '{key}' has status '{value}', which is not a valid "
                f"{'planning placeholder' if RETRO_KEY.match(key) else 'lifecycle state'} "
                f"(allowed: {sorted(allowed)})"
            )
    if not in_section and not entries:
        problems.append("no development_status section found")
    return entries, problems


def check(entries):
    problems = []
    epic_statuses = {}  # epic number -> status of the epic-N key, when present
    stories_by_epic = {}  # epic number -> [(story key, status)]
    for key, value in entries:
        epic_match = EPIC_KEY.match(key)
        if epic_match:
            epic_statuses[int(epic_match.group(1))] = value
            continue
        story_match = STORY_KEY.match(key)
        if story_match:
            stories_by_epic.setdefault(int(story_match.group(1)), []).append((key, value))

    for number in sorted(set(epic_statuses) | set(stories_by_epic)):
        epic_status = epic_statuses.get(number)
        stories = stories_by_epic.get(number, [])
        if epic_status is None or not stories:
            continue  # nothing to cross-check against
        non_terminal = [(k, v) for k, v in stories if v not in TERMINAL]
        if epic_status == "done" and non_terminal:
            for key, value in non_terminal:
                problems.append(
                    f"epic-{number} is 'done' but story key '{key}' is '{value}' — the file "
                    f"reports outstanding work the epic is already closed from"
                )
        elif epic_status != "done" and stories and not non_terminal:
            problems.append(
                f"every story of epic-{number} is terminal "
                f"({', '.join(v for _, v in stories)}) but the epic key is '{epic_status}' — "
                f"fully accepted work under an epic the file still says is open"
            )
    return problems


def main(argv):
    path = argv[1] if len(argv) > 1 else "docs/bmad/implementation-artifacts/sprint-status.yaml"
    try:
        with open(path, encoding="utf-8") as handle:
            text = handle.read()
    except OSError as err:
        print(f"error: cannot read {path}: {err}", file=sys.stderr)
        return 2

    entries, format_problems = parse_development_status(text)
    problems = format_problems + check(entries)

    if problems:
        print(f"sprint-status: {len(problems)} violation(s) in {path}", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(f"sprint-status: {len(entries)} keys consistent in {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
