---
name: zhoubao-skill
description: Summarize only the user's own Git commits and P4 changelists from the most recent 7 days, then write a weekly report focused on problems solved, actions taken, and results. Use when the user asks for weekly report, weekly progress, or "what issues I solved this week" based on Git or Perforce history.
---

# 周报 Skill

## 适用范围

- 必须得到用户确认的 Git / P4 工作目录路径，才能统计。
- 默认同时统计当前 Git 仓库与当前可访问的 P4（Perforce）记录。
- 如果用户明确说只看 Git 或只看 P4，按用户要求收窄范围。

## 固定规则

- 找不到git或者p4目录时，一定要先询问用户，输入Git工作目录或者P4工作目录，如果用户不提供，则按照当前目录生成。
- 只统计最近 7 天。
- 只统计用户本人变更。
- Git 统计单位是 commit，P4 统计单位是 submitted changelist。
- 默认直接输出周报内容，不输出大段命令过程，除非用户要求。
- 默认只输出“本周解决的问题”正文，不额外输出时间范围、统计概览或风险小节。

## 执行流程
0. 确认目录
- 当前目录没有git或者p4仓库之一，必须应该先询问用户，等待输入P4工作目录或者Git工作目录，或者让用户切换到对应目录后再执行周报生成。

1. 确认本人身份
- Git：优先读取 `git config user.email`，为空时回退 `git config user.name`。
- Git：所有 `git log` / `git show` 查询都必须加 `--author="..."`。
- P4：优先读取 `p4 set P4USER`，拿不到再读 `p4 info` 里的 `User name`。
- P4：所有 changelist 查询都必须加 `-u user`。

2. 固定计算时间窗口（最近 7 天）
- 使用当前本地时间 `now`。
- `start = now - 7 days`，`end = now`。
- 时间范围仅用于内部筛选最近 7 天的数据，不默认写入周报正文。
- Git 直接用 `--since` / `--until` 限制时间。
- P4 用 `p4 changes -t` 拉取最近 changelist 后，在本地按 changelist 时间二次过滤到 `[start, end]`。

3. 收集证据
- Git：用 `--name-status` 看改动范围。
- Git：用 `--shortstat` 与 `--numstat` 统计提交数、增删行。
- Git：对关键提交用 `git show` 抽取“改了什么问题”，不要贴整段 diff。
- P4：用 `p4 changes -t -s submitted -u ...` 收集最近 changelist。
- P4：用 `p4 describe -s` 看描述和影响文件。
- P4：用 `p4 describe -ds` 看 diff summary，并据此统计新增/删除行。
- 如果 `p4 changes -m 200` 仍不足以覆盖最近 7 天，要继续提高上限，不要漏数据。

4. 产出周报
- 按“问题 -> 动作 -> 结果”写。
- 同一主题的多个 Git commit / P4 changelist 要合并成一条“已解决问题”。
- 每条问题必须用一句完整的话写完，单独占一行，不拆成多级结构。
- 默认不输出标题、编号、统计概览、时间范围或风险项，直接逐行输出问题。
- 语言跟随用户；用户用中文就输出中文。

## 命令模板（PowerShell）

身份与时间窗口：

```powershell
$gitAuthor = git config user.email
if (-not $gitAuthor) { $gitAuthor = git config user.name }

$p4UserLine = p4 set P4USER 2>$null | Select-Object -First 1
if ($p4UserLine -match '^P4USER=(.+?)\s') { $p4User = $matches[1] }
if (-not $p4User) {
  $p4Info = p4 info 2>$null | Select-String '^User name:\s+(.+)$'
  if ($p4Info) { $p4User = $p4Info.Matches[0].Groups[1].Value.Trim() }
}

$now = Get-Date
$start = $now.AddDays(-7)
$startText = $start.ToString("yyyy-MM-dd HH:mm:ss")
$endText = $now.ToString("yyyy-MM-dd HH:mm:ss")
```

Git 证据：

```powershell
git log --author="$gitAuthor" --since="$startText" --until="$endText" --pretty=format:"%h|%ad|%s" --date=iso
git log --author="$gitAuthor" --since="$startText" --until="$endText" --name-status --pretty=format:"=== %h|%ad|%s"
git log --author="$gitAuthor" --since="$startText" --until="$endText" --shortstat --pretty=format:"=== %h|%ad|%s"
```

Git 增删行汇总：

```powershell
$s = git log --author="$gitAuthor" --since="$startText" --until="$endText" --numstat --pretty=tformat: | Select-String '^(\d+)\s+(\d+)\s+'
$gitIns = 0
$gitDel = 0
foreach($m in $s){
  $p = $m.Line -split '\s+'
  $gitIns += [int]$p[0]
  $gitDel += [int]$p[1]
}
"git_insertions=$gitIns git_deletions=$gitDel"
```

P4 changelist 列表与时间过滤：

```powershell
$p4Raw = p4 changes -t -s submitted -u "$p4User" -m 200
$p4Changes = foreach ($line in $p4Raw) {
  if ($line -match "^Change\s+(\d+)\s+on\s+(\d{4}/\d{2}/\d{2})\s+(\d{2}:\d{2}:\d{2})\s+by\s+.+?\s+'(.*)'$") {
    $ts = Get-Date "$($matches[2]) $($matches[3])"
    if ($ts -ge $start -and $ts -le $now) {
      [pscustomobject]@{
        Change = [int]$matches[1]
        Time = $ts
        Subject = $matches[4]
      }
    }
  }
}
$p4Changes
```

P4 详细信息与增删行汇总：

```powershell
$p4Ins = 0
$p4Del = 0
foreach ($cl in $p4Changes) {
  p4 describe -s $cl.Change

  $summary = p4 describe -ds $cl.Change
  foreach ($line in $summary) {
    if ($line -match '^add \d+ chunks (\d+) lines$') {
      $p4Ins += [int]$matches[1]
    } elseif ($line -match '^deleted \d+ chunks (\d+) lines$') {
      $p4Del += [int]$matches[1]
    } elseif ($line -match '^changed \d+ chunks (\d+) / (\d+) lines$') {
      $p4Del += [int]$matches[1]
      $p4Ins += [int]$matches[2]
    }
  }
}
"p4_insertions=$p4Ins p4_deletions=$p4Del"
```

## 周报输出模板

- 直接输出问题列表，不输出标题、编号或其他额外说明。
- 每个问题必须是一句完整的话，写完后直接换行写下一个问题。
- 一句话内同时交代问题、采取的动作和结果，不附带来源信息。
示例：
- 调整弹射计数逻辑消除了重复触发，修复了 PRT 弹射次数异常问题，弹射次数恢复正确。
- 补齐风场相关配置和验证流程提高了场景表现一致性，修复了风场表现不稳定的问题，关测试可以稳定复现预期效果。

## 质量检查

提交前检查：

- 是否严格限制在最近 7 天。
- 是否严格限制为用户本人 Git commit 与 P4 changelist。
- 每条“解决的问题”是否都用一句完整的话单独成行。
- 输出里是否没有额外的标题、编号、统计概览、时间范围或风险说明。
