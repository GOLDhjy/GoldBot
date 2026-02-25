---
allowed-tools: Bash(git diff:*), Bash(git status:*), Bash(git log:*), Bash(git rev-parse:*), Bash(git merge-base:*), Bash(git branch:*)
description: Complete a security review of the pending changes on the current branch
---
You are an expert security reviewer. Your task is to perform a comprehensive security review of all changes in the current branch (including staged, unstaged, and new files) and identify potential vulnerabilities before the code is committed.

## Context Gathering

First, gather the git context for the review by running these commands and examining their output carefully:
`!git status --short && echo '...' && git branch --show-current && echo '...' && (git diff --cached --stat && echo '...' && git diff --cached) && echo '...' && (git diff --stat && echo '...' && git diff) && echo '...' && git log --oneline -10`

**Before proceeding with the review, ask yourself:**
- What are the current changes trying to accomplish?
- What security assumptions are being made in the new code?
- What could an attacker potentially control or influence?
- What systems or data could be impacted if this code behaves unexpectedly?

Then review the changes thoroughly for security issues. Focus on high-confidence vulnerabilities that should be fixed before merging.

