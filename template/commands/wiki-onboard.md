---
description: Generate an onboarding reading path through the wiki for a specific reader profile.
argument-hint: --profile <"backend dev" | "data engineer" | "PM" | "on-call" | ...>
allowed-tools: Read, Glob
---

Use the @wiki-onboarder subagent to generate a tailored reading path. Profile from $ARGUMENTS.

Output: a Markdown numbered list of 5–10 page slugs with 1-line rationales.
