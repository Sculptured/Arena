# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue:

- **Preferred:** [open a private vulnerability report](https://github.com/Sculptured/Arena/security/advisories/new) on GitHub.
- **Or email** security@usesculptured.com.

Include what you found, how to reproduce it, and the commit you tested. You will get an acknowledgement within three working days. We will keep you updated until a fix is released, and credit you if you wish.

## Scope

Shakedown runs AI agents on your own computer, against systems it starts for them, with your own Claude Code and Codex sign-in or OpenRouter key. Reports we especially want:

- A way for a web page, another local user or a remote party to reach arenad's `/api` routes (arenad listens on 127.0.0.1 and answers `/api` only for a loopback `Host`), or to drive runs without the user.
- A CLI agent getting more than arenad gives it: Claude Code using a tool other than the world's tools and `handoff`, Codex writing outside its read-only sandbox or opening a network socket, either CLI loading the user's own configuration, or reading the database credentials and API key that arenad removes from its environment.
- A package with an `image` run escaping its container: network access its manifest did not ask for, environment from arenad, or files outside its own saves folder.
- A save, restore or garbage collection reading, overwriting or deleting data outside what arenad and its integrations made.
- Secrets (`OPENROUTER_API_KEY`, database passwords) leaking into recorded events, logs or the browser.

Not vulnerabilities on their own:

- A package with a `command` run can do anything your user can; install only packages you trust.
- Codex's built-in shell cannot be turned off. It runs in a read-only sandbox; a way out of that sandbox is in scope.
- Agents act with the tools the benchmark gives them: an agent that makes a bad write through a world tool is the behaviour Shakedown exists to record.

## Supported versions

Shakedown has no releases yet. Security fixes go to `main`.
