---
name: security-review
description: Security review workflow adapted from Anthropic's .claude/commands/security-review.md. Use when the user asks for /security-review, a security review of pending changes/current branch/PR diff, or a high-confidence vulnerability-focused review before commit or merge.
---

# Claude Security Review

Use this skill to perform a security-focused review of code changes, with emphasis on high-confidence vulnerabilities and low false positives.

## Workflow

1. Default to reviewing pending changes in the current branch:
   - staged changes (`git diff --cached`)
   - unstaged changes (`git diff`)
   - newly added tracked files
2. Gather git context first using read-only commands.
3. Review code paths and trust boundaries, not just syntax.
4. Report only high-confidence security issues that should be fixed before merge.
5. Do not change code unless the user explicitly asks for remediation.

## Context Gathering

Run these commands (separately is fine) and inspect the output carefully:

```powershell
git status --short
git branch --show-current
git diff --cached --stat
git diff --cached
git diff --stat
git diff
git log --oneline -10
```

If the repository uses Perforce (P4) instead of Git, use this rough equivalent set (there is no exact `branch` / `--cached` equivalent):

```powershell
# git status --short
p4 opened
p4 reconcile -n
# (if supported by your P4 version)
p4 status
# git branch --show-current (closest: workspace / stream)
p4 info
p4 client -o | Select-String '^Client:|^Stream:'
# git diff --cached --stat (approx: pending changelist file list)
p4 opened -c <changelist>
# git diff --cached (approx: pending changelist diff)
p4 diff -du -c <changelist>
# git diff --stat (no direct equivalent; use file list + diff)
p4 opened
p4 diff -du
# git diff
p4 diff -du
# git log --oneline -10
p4 changes -m 10
```

If reviewing a shelved changelist in P4, use:

```powershell
p4 describe -S -du <changelist>
```
If the user asks for PR/base-branch review instead of "pending changes", compute a merge base and review that diff:
```powershell
git branch --show-current
git rev-parse --verify origin/main
git merge-base HEAD origin/main
git diff --stat <merge-base>...HEAD
git diff <merge-base>...HEAD
```

If `origin/main` does not exist, try `origin/master`, then ask the user which base branch to use.

## Review Standard

Think through these questions before reporting findings:

- What are the changes trying to accomplish?
- What inputs can an attacker control?
- What trust boundaries are crossed?
- What secrets, systems, or data could be impacted?
- What assumptions are implicit, and are they enforced?

Focus on exploitable or clearly unsafe issues such as:

- injection (SQL/command/template/code)
- authn/authz bypass or privilege escalation
- insecure deserialization or dynamic code execution
- SSRF / open redirect / path traversal / file write-read abuse
- secret exposure / credential leakage
- weak crypto usage or disabled verification
- unsafe shelling out, temp file handling, or permission mistakes
- sandbox/tenant isolation breaks

Do not report:

- style issues
- generic best-practice advice without a concrete vulnerability
- speculative concerns without a realistic attack path
- non-security bugs unless they directly create a security impact

## Output Format

使用中文输出
Return findings first (highest severity first). If none exist, state:

`没有在审查的更改中发现高置信度的安全问题。`

For each finding, include:

- Severity
- Confidence
- Location (`path:line` when possible)
- Vulnerability summary
- Why it is exploitable / impact
- Recommended fix (concrete)

Keep the report concise and evidence-based.

## Reference Source

Read `references/security-review.md` when you need the original Anthropic slash-command wording or want to compare behavior exactly.

