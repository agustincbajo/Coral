# Audit findings — 2026-05-11

Each finding lives in `findings/NNN-slug.md` with a fixed frontmatter so we can
batch-create issues with `gh issue create` once the auditor is authenticated.

## Status

| ID  | Title                                          | Severity | Confirmed | Issue |
|-----|------------------------------------------------|----------|-----------|-------|
| 001 | Windows-GNU build fails without `dlltool.exe`  | Low      | yes       | —     |

## How to file the issues

```powershell
gh auth login
foreach ($f in Get-ChildItem audit/findings/*.md) {
  $body = Get-Content $f.FullName -Raw
  $title = ($body | Select-String '^title:\s*(.+)$').Matches[0].Groups[1].Value
  $labels = ($body | Select-String '^labels:\s*(.+)$').Matches[0].Groups[1].Value
  gh issue create --title $title --body $body --label $labels
}
```
