---
name: web-research
description: Web search and page-fetch guidance for this machine — choosing between the built-in tools and the self-hosted SearxNG, plus measured per-engine verdicts and the parameters that actually change results. Use whenever searching the web, looking up API or library documentation, researching an error message, comparing versions, or fetching a page.
---

# Web research

Four tools, two providers. They fail in opposite directions, so the choice matters.

| Tool | What it does |
|---|---|
| `WebSearch` | Retrieval **+ synthesis**. Returns a written answer. US-only, no time filter. |
| `WebFetch` | Fetches one page, answers a prompt against it with a small model. |
| `mcp__searxng__searxng_web_search` | Retrieval **only** — ranked links, snippets, relevance scores. Self-hosted at `localhost:8080`. |
| `mcp__searxng__web_url_read` | Fetches one page as raw markdown. No synthesis. |

## Which to reach for

**Default to `WebSearch`.** One call usually answers the question, often with working code.

**Switch to SearxNG when:**

- **Version or recency matters** — pass `time_range: 'year'`/`'month'`. `WebSearch` has no such filter and will happily summarize a stale release as current.
- **The built-in returned nothing useful.** A weak result is one provider's ranking, not evidence of absence. Re-run on SearxNG before concluding the answer doesn't exist.
- **Synthesis might hide the detail you need.** Measured: asked about Rust 1.96, `WebSearch` summarized 1.96.0 and never mentioned that 1.96.1 existed with three CVE fixes. Raw ranked results surfaced it immediately. A tidy paragraph reads as complete whether or not it is.
- **You want canonical docs over SEO chaff.** On an `.mrpack` spec query, half the built-in results were converter-tool spam; SearxNG ranked the official docs first.

**Use `WebFetch`, not `web_url_read`, for targeted questions about a page.** `web_url_read` returns raw markdown *from the top*, and nav sidebars routinely eat the whole budget — on a docs site it returned nothing but a version list. Reach for `web_url_read` only when you want unsummarized text, or use its `section` / `readHeadings` / `paragraphRange` parameters.

## The single highest-value parameter

**Pass `categories: 'it'` for technical questions.** SearxNG routes by category — only engines declaring the queried category fire, so the instance-wide engine count is irrelevant to any one query.

Measured on this instance, same query:

- default (`general`) → **google cse + duckduckgo** only
- `categories: 'it'` → **stackoverflow, mdn, github, docker hub**, zero unresponsive

The two sets are disjoint. Running a dev question on the default category is close to the worst available configuration.

## Engine selection

A `disabled` engine **still answers an explicit `engines:` request** — `disabled` only removes it from the defaults. No config change is needed to use anything below.

| `engines:` value | Verdict |
|---|---|
| `crates.io` | **Use** for Rust crates. Focused, exact crate first, real descriptions. |
| `hackernews` | **Use** for design/opinion questions. Consistently on-topic threads. |
| `yep` | **Use.** Independent index — real diversity vs Google/Bing, consistent across queries. |
| `duckduckgo web` | **Use.** Most consistent of all general engines; lands canonical docs first. |
| `stackoverflow`, `mdn`, `github`, `docker hub` | Solid; these are what `categories: 'it'` already gives you. |
| `lib.rs` | Discovery only — 144 results for the one-word query `tokio`. Drowns co-queried engines. |
| `bing` | Usable but ambiguity-prone: `rust lifetime elision` returned the *video game* first. |
| `npm` | **Avoid in Rust queries** — see below. |
| `gitlab` | Marginal — bare repo names, no context. |
| `mwmbl`, `mojeek` | Unreliable — 0 results on half the test queries. |
| `qwant`, `presearch`, `codeberg`, `reddit` | Dead on this instance (access denied / timeout). |

### Two silent traps

Both return confident nonsense rather than an obvious error, which is why they are worth naming:

- **`npm` on a Rust query.** Rust and npm share package names. Searching `tokio` returns *"tokio | Web scraping made simple"* — an unrelated JavaScript package, ranked first, indistinguishable from a real hit. This repo is a Cargo workspace *and* a Node frontend, so the collision is live. Scope `npm` to JS-only lookups.
- **`categories: 'social media'`.** Matches Mastodon *usernames* against query tokens — `tauri vs electron memory usage` returned accounts named "vs", "memory", "tauri".

### Reddit

The `reddit` engine returns `access denied` (post-2023 API lockdown). **Append `reddit` to a normal `general` query instead** — roughly 17 of 28 results come back as on-topic threads. This is also just better: you get Google's ranking of Reddit rather than Reddit's own, which has always been weak.

## Instance notes

- Config lives at `C:\Users\jarja\searxng\searxng\settings.yml` (`use_default_settings: true`, so `engines:` entries **merge by name** — list only `name` plus the keys you're changing).
- Runs as the `searxng` Docker container. **Settings are read at startup only** — `docker restart searxng` after editing.
- Preferences set in the SearxNG *web frontend* are cookie-scoped and invisible to the API. They will not affect these tools.
- `brave` and `startpage` are persistently suspended upstream (rate-limit / CAPTCHA). Check
  `unresponsive_engines` in a raw `/search?format=json` response when results look thin.
