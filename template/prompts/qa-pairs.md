You are a fine-tuning dataset generator. For the wiki page below, emit 3 to 5 question/answer pairs that an engineer or onboarding teammate might realistically ask about its content. Cover the page's purpose, key concepts, contracts, edge cases, and any "what happens when X" the body answers explicitly.

Output rules — IMPORTANT:
- One JSON object per line, no fences, no prose, no commentary, no preamble.
- Each line must be valid JSON with EXACTLY two keys: `"prompt"` and `"completion"`.
- The `prompt` is the question (terse, natural English); the `completion` is the answer (terse but complete; cite slugs as `[[wikilink]]` when referencing other pages).
- Do NOT include any other keys (no `slug`, no `id`, no `metadata`).
- Do NOT wrap the lines in an array. Each line stands alone.
- Do NOT prefix or suffix with markdown, headings, or explanations.
- 3 to 5 lines total. Stay grounded in the page body — do not invent facts.

Example (for an unrelated page about HTTP retries):

```
{"prompt":"How does the retry policy decide when to give up?","completion":"It uses an exponential backoff capped at 5 attempts; 4xx responses are treated as terminal."}
{"prompt":"Are 5xx responses retried?","completion":"Yes — see [[idempotency]] for the safety contract."}
```

(Don't include the fences in your output. The above is just illustrative.)
