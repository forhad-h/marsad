# Output hygiene

## Keeping my request out of the deliverable

Every message from me has two layers:

1. **The deliverable** — what I want produced.
2. **Working context** — why I want it, who it's for, my situation, and which skills, tools, or workflows to use.

Working context exists to shape *how* you work and *how the result reads* (tone, depth, format, what to explain vs. assume). It is never itself part of the result. "For junior engineers" means write more explanation; it does not mean write "this is for junior engineers."

### The test

Every artifact you produce — code, comments, docstrings, docs, configs, commit messages, PR titles and descriptions, reports, filenames — must read correctly to someone who:

- never saw my message,
- doesn't know who asked for the work or why,
- doesn't have my machine, accounts, installed tools, or skills,
- doesn't know an assistant was involved.

Describe the work. Don't describe the request, the requester, or the production process.

### What leaks, specifically

Remove anything like:

- Acknowledgement or framing: "As requested", "Per your instructions", "Here is the…", "You asked for…"
- Process narration: "Used the X skill", "Generated with…", "Verified using Y tool" — unless that tool is a declared project dependency.
- My stated reasons or situation: "because the client meeting is Friday", "since you're migrating off Z".
- People from my message. Someone belongs in the output only if they're part of the work's own record (an author, reviewer, or owner the project tracks), not because I named them.
- References to this conversation, earlier turns, or my local setup.
- In commit messages and PR descriptions: say what changed and why it matters to the codebase, never who asked or how it was asked.

### Where context *is* fine

Your chat reply to me is not the deliverable. There you can and should mention what you did, which tools you used, and what you left out. The boundary is the artifact itself.

### Before finishing

Reread each artifact as a teammate seeing it cold. Anything that only makes sense with my message beside it gets cut.

If a fact could be either a project fact or a request fact and you can't tell which, treat it as a request fact and leave it out. If it's actually needed, I'll ask for it explicitly.
