---
name: security-review
description: Security review workflow adapted from Anthropic's .claude/commands/security-review.md. Use when the user asks for /security-review, a security review of pending changes/current branch/PR diff, or a high-confidence vulnerability-focused review before commit or merge.
---

# Claude Security Review

Use this skill to perform a security-focused review of code changes, with emphasis on high-confidence vulnerabilities and low false positives.

For Unreal Engine projects, strengthen checks for C++ and Lua changes.

## Workflow

1. Default to reviewing pending changes in the current branch:
   - staged changes (`git diff --cached`)
   - unstaged changes (`git diff`)
   - newly added tracked files
2. Gather git context first using read-only commands.
3. Review code paths and trust boundaries, not just syntax.
4. Report only high-confidence security issues that should be fixed before merge.
5. Do not change code unless the user explicitly asks for remediation.
6. If this is a UE project, prioritize changed C++/Lua files and apply the enhanced checks below.

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

### UE C++ Enhanced Checks

When changed files include UE C++ (`.h/.hpp/.cpp/.cxx`), add these mandatory checks:

- C++ high-risk bug classes (must prioritize):
  - Null pointer dereference / wild pointer dereference.
  - Dangling pointer / use-after-free / double free.
  - Memory/resource leaks (heap/object/handle/timer/delegate/thread task).
  - Buffer/index out-of-bounds (`TArray`, raw arrays, string formatting/copy).
  - Integer overflow/underflow and signed/unsigned conversion causing size/index bugs.
  - Lifetime mismatch in async callbacks (object destroyed but callback still captures `this`).
- Pointer and ownership discipline:
  - Verify ownership model is explicit (`TUniquePtr`/`TSharedPtr`/`TWeakObjectPtr`/raw ptr usage).
  - For `UObject` pointers, check GC-safety and validity checks before use.
  - Check unsafe casts, invalid downcasts, and missing null checks after cast/find/get operations.
- Input validation and trust boundaries:
  - Validate all client/network/file inputs (length, range, enum, state, nullability).
  - Any state-changing path must enforce server-side permission checks.
  - For UE RPC (`UFUNCTION(Server, ...)` etc.), ensure no direct trust in client payload.
- Concurrency and async safety:
  - Shared state races, missing synchronization, and callback-after-destroy patterns.
  - Verify game-thread assumptions vs worker-thread access for mutable engine/game objects.
  - Check lock misuse/deadlock risk and non-thread-safe container/object access.
- Serialization/deserialization and reflection:
  - Untrusted JSON/binary/archive input must have schema/range checks before use.
  - Dynamic class/object loading and reflection-driven invocation must be constrained.
- File system and process execution:
  - Path traversal/arbitrary read-write risks in `FFileHelper`/`IPlatformFile`/`IFileManager`.
  - Command injection risk in process launch (`FPlatformProcess::CreateProc`) and arg concatenation.
- Authentication/authorization and privilege surface:
  - Privileged operations require explicit role/session checks.
  - Debug/cheat/admin code paths must not be reachable in shipping gameplay flows.
- Network, crypto, and secret handling:
  - No disabled TLS verification, weak crypto downgrade, or hardcoded credentials/tokens.
  - Avoid leaking secrets or sensitive IDs via logs/error messages.

### Lua Enhanced Checks

When changed files include Lua (`.lua`), add these mandatory checks:

- Dynamic execution:
  - `load`/`loadstring`/`dofile`/dynamic `require` with user-influenced input.
- Command and file abuse:
  - `os.execute`/process wrappers and `io.*` file access with unsanitized path/input.
- Trust boundary:
  - Event/RPC handlers missing permission checks or trusting client payload directly.
- Injection and leakage:
  - String-template code injection patterns, sensitive data in logs/errors.
- Sandbox escape risk:
  - Exposure of unsafe globals/libraries (`debug`, unrestricted `_G`, etc.) in runtime scripts.

Do not report:

- style issues
- generic best-practice advice without a concrete vulnerability
- speculative concerns without a realistic attack path
- non-security bugs unless they directly create a security impact

## Output Format

使用中文输出。
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
