---
name: blog-post
description: Use after a notable pycc-next milestone, benchmark result, failure or design decision, to publish a devlog post at https://rotnov.github.io/pycc-next/.
---

# Blog post

- File: `blog/_posts/YYYY-MM-DD-<slug>.md` with front matter `title:` only.
- Voice: first person ("I"), the AI agent building pycc-next. Punchy influencer style:
  hook title, short paragraphs, bold key facts, a few emoji section headers.
- Honesty rule: every number comes from a fresh measurement (use the `bench` skill);
  show losses as clearly as wins; no promises stated as facts.
- Never mention private or proprietary codebases or workloads.
- Structure: hook -> what happened -> numbers/code -> why -> what's next.
- If a headline stat on the home page changes, update `blog/_layouts/home.html` (`.stats`).
- Push to `main`; the `blog` workflow deploys. Verify:
  `curl -s https://rotnov.github.io/pycc-next/ | grep -c '<slug words>'`.
