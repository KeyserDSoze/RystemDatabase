
# Immutable GitHub Actions dependencies (#67)

CI and release use GitHub-hosted third-party actions at reviewed **full
40-character Git commit SHAs**, not mutable version tags. Relative local
actions remain bound to the checked-out repository commit.

## Official upstream review — 2026-11-22

These SHA values were verified against GitHub REST endpoints
git/ref/tags/<major> in the respective **official action repositories**.
Each tag ref returned a commit object with the SHA below. The major version
is preserved in YAML comments for human review only.

| Action repository | Major tag | Verified full commit SHA |
| --- | --- | --- |
| actions/checkout | v4 | 11d5960a326750d5838078e36cf38b85af677262 |
| actions/setup-node | v4 | 49933ea5288caeca8642d1e84afbd3f7d6820020 |
| actions/setup-dotnet | v4 | 67a3573c9a986a3f9c594539f4ab511d57bb3ce9 |
| actions/upload-artifact | v4 | ea165f8d65b6e75b540449e92b4886f43607fa02 |
| actions/download-artifact | v4 | d3f86a106a0bac45b974a628896c90dbdf5c8093 |
| docker/login-action | v3 | c94ce9fb468520275223c153574b00df6fe4bcc9 |
| docker/build-push-action | v6 | 10e90e3645eae34f1e60eeb005ba3a3d33f178e8 |

**Coverage:** 18 action references in two workflow files (6 CI and 12
release). Repeated use of one action shares the reviewed commit SHA.

## Maintenance

1. Inspect official action release notes, code differences, permissions
   and Node/runtime compatibility, not just the current moving tag.
2. Verify the proposed full SHA in the official upstream GitHub repo and
   record the reviewed update in a GitHub issue.
3. Change ALL references to that action in both workflows and keep its
   version comment readable; never use a mutable or abbreviated ref.
4. Run these two deterministic, network-free checks, followed by the
   full CI/release cross-platform verification:
   - python3 scripts/check_workflow_action_pins.py --self-test
   - python3 scripts/check_workflow_action_pins.py

## Security limitations

The Python stdlib script checks the simple YAML uses syntax currently in
this repository, recognizes repository-relative local actions, and rejects
unrecognized external forms. It requires **no PyPI dependencies, shell
installation or network calls during CI**. This is a pinning control only:
it does not cryptographically attest upstream commits, analyze source
code in actions, vet transitive dependencies, scan secrets, prove release
integrity or prevent all possible supply-chain attacks. Additional
credential, dependency, formatting and fuzzing gates remain tracked by
[#11](https://github.com/KeyserDSoze/RystemDatabase/issues/11).
