#!/usr/bin/env python3
"""Enforce immutable, reviewed GitHub Actions uses in every workflow.

No YAML/PyPI/network dependency. This deliberately checks only the restricted
'uses:' syntax in this repository. It is a supply-chain *pinning* gate, not
an action security audit or verification of upstream commit provenance.
"""
from __future__ import annotations

import pathlib
import re
import sys
import unittest

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOWS = REPO_ROOT / ".github" / "workflows"
USES = re.compile(r"^\s*(?:-\s*)?uses:\s*(.*)$")
# An explicitly reviewed lower-case 40-character Git commit (not a tag,
# moving branch, truncated SHA, or a runtime expression).
PINNED = re.compile(
    r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+"
    r"(?:/[A-Za-z0-9_.-]+)*@[0-9a-f]{40}\Z"
)


def find_violations(name: str, source: str) -> list[str]:
    violations: list[str] = []
    for line_number, line in enumerate(source.splitlines(), start=1):
        match = USES.fullmatch(line)
        if match is None:
            continue
        # Strip only an inline YAML comment introduced by whitespace.
        ref = match.group(1).split(" #", 1)[0].strip().strip("'\u0022")
        # Relative local actions are source-controlled by this repo's commit.
        if ref.startswith("./") and ".." not in pathlib.PurePosixPath(ref).parts:
            continue
        if PINNED.fullmatch(ref) is None:
            violations.append(
                f"{name}:{line_number}: unpinned/unsupported action reference "
                f"{ref!r}; expected owner/repo@<40 lower-case hex SHA>"
            )
    return violations


def check_repository() -> int:
    paths = sorted((*WORKFLOWS.glob("*.yml"), *WORKFLOWS.glob("*.yaml")))
    if not paths:
        print("No workflow files found: cannot validate pinning", file=sys.stderr)
        return 1
    violations: list[str] = []
    for path in paths:
        violations.extend(
            find_violations(path.relative_to(REPO_ROOT).as_posix(),
                            path.read_text(encoding="utf-8"))
        )
    if violations:
        print("\n".join(violations), file=sys.stderr)
        return 1
    print(f"Action SHA pin gate: PASS ({len(paths)} workflow files)")
    return 0


class PinGateTests(unittest.TestCase):
    SHA = "0123456789abcdef" * 2 + "01234567"

    def test_pinned_action_with_auditable_major_comment(self) -> None:
        self.assertEqual(
            find_violations("ci.yml", f"  - uses: actions/checkout@{self.SHA} # v4"),
            [],
        )

    def test_major_tags_branches_and_short_shas_fail(self) -> None:
        for suffix in ("v4", "main", "0123456", "0123456789abcdef" * 2):
            with self.subTest(suffix=suffix):
                self.assertEqual(len(find_violations(
                    "release.yml", f" - uses: actions/checkout@{suffix}")), 1)

    def test_dynamic_and_image_sources_do_not_sneak_through(self) -> None:
        for line in (
            "  uses: $" + "{{ inputs.action }}",
            "  - uses: docker://alpine:latest",
            "  - uses: actions/checkout",
            "  - uses: actions/checkout@DEADBEEF" + "01234567" * 4,
        ):
            with self.subTest(line=line):
                self.assertEqual(len(find_violations("ci.yml", line)), 1)

    def test_quotes_indentation_and_local_actions(self) -> None:
        self.assertEqual(find_violations(
            "ci.yml",
            f"""    - uses: 'actions/checkout@{self.SHA}' # v4
      uses: "./.github/actions/local-check"
# - uses: actions/malicious@main
"""), [])

    def test_errors_pinpoint_workflow_line(self) -> None:
        violations = find_violations("ci.yml",
            "name: ci\nsteps:\n  - uses: actions/checkout@v4\n")
        self.assertEqual(len(violations), 1)
        self.assertTrue(violations[0].startswith("ci.yml:3:"))


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(PinGateTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        sys.exit(0 if result.wasSuccessful() else 1)
    if len(sys.argv) != 1:
        sys.exit("Usage: check_workflow_action_pins.py [--self-test]")
    sys.exit(check_repository())
