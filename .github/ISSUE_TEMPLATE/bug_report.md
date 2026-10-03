---
name: Bug report
about: Create a report to help us improve
title: ''
labels: ''
assignees: ''

---

**Describe the bug**
A clear and concise description of what the bug is.

**To Reproduce**
Steps to reproduce the behavior:
1. Open '...' (quick search, the library window, `shouci …`, `shouci-tui`)
2. Type or click '...'
3. See error

For search results, include the exact query and what you expected to find
(the characters, if you know them).

**Expected behavior**
A clear and concise description of what you expected to happen.

**Screenshots**
If applicable, add screenshots to help explain your problem.

**Your setup (please complete the following information):**
 - Where it happened: Mac app / `shouci` / `shouci-tui`
 - macOS version: [e.g. 15.1]
 - Shouci commit or version: [`git log -1 --oneline` in your checkout, or Shouci › About Shouci with the library open]
 - Output of `./scripts/install.sh --status`, if it's about installing

**Additional context**
Add any other context about the problem here. Errors from the app are in
the system log: `log show --last 10m --predicate 'subsystem == "com.zacharyzampa.shouci"'`.
Shouci never logs what you search.
