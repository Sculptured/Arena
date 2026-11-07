# Local pharma stack

Runs arenad with the pharma benchmark (Larchmont Biologics) on http://127.0.0.1:4513, serving the UI
from `ui/dist`. Build the UI first (`cd ui && npm run build`), then:

```bash
examples/pharma-stack/serve.sh     # (re)starts arenad detached; log in $ARENA_DEMO_LOG
```

Agents are real models. Claude Code and Codex work through their logged-in CLIs; API providers
(OpenRouter, OpenAI, Anthropic) need their keys in the environment before `serve.sh`.
