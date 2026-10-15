# Working in this repository

`ikigai-web-demo` runs the ikigai resolution kernel in the browser as WebAssembly: a page the
kernel composes, with an in-page terminal and a guided runbook, published to GitHub Pages, plus a
WebTransport server that serves the same page over the network. These rules are for every agent
and every person working here. `CLAUDE.md` is a link to this file.

- **Read [ACCESSIBILITY.md](ACCESSIBILITY.md) before any change to UI or rendered output**:
  `dist/index.html`, `dist/net.html`, and the page shapes and catalog stylesheet in `src/lib.rs`.
  It lists what has been tested, what has not, and the barriers known today. The page exists only
  once the kernel composes it, so check it in a browser.
- Security reports go through the organization's security policy (GitHub private vulnerability
  reporting), never a public issue.
- Every change lands through a pull request with green CI: `.github/workflows/ci.yml` (fmt,
  clippy with warnings denied, the wasm library's clippy, tests and docs) and
  `.github/workflows/pages.yml` (the wasm build and the smoke suite over the tree it deploys).
- Never invent testing, tool results or conformance evidence, and never weaken a test to make a
  change pass.
- Use American spelling in everything you write: code comments, docs, commit messages and pull
  requests.
