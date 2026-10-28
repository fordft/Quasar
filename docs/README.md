# Quasar documentation

Code and tests are the evidence of present behavior. Where documents appear to
disagree, `security-model.md` controls claims about enforced security,
`architecture.md` controls the browser foundation, and `product-system.md`
controls product intent.

## Architecture and security

- [Architecture](architecture.md): implemented browser structure, domain ownership and native lifecycle.
- [Security model](security-model.md): the guarantees the tree enforces and the gates for disabled capabilities.
- [Product system](product-system.md): what Quasar is, the first-release scope and the target architecture.
- [Frontend contract](frontend.md): stack, ownership, import boundaries and native presentation rules for `frame/`.

## Browser features

- [Interface design](design/interface.md): why the browser chrome is shaped the way it is.
- [Interface system](design/system.md): tokens, shared components and how to inspect them.
- [Settings](design/settings.md): settings structure, search and navigation.
- [Launcher](design/launcher.md): the floating search window and what it hands off.
- [History](design/history.md): the history module, omnibox queries and site icons.
- [Tasks](design/tasks-redesign.md): the shared Tasks surface for people and agents.
- [Charts](design/charts.md): the chart kit used by Work results and Activity.
- [File workflows](file-workflows-windows-qa.md): upload and download ownership, recovery, and Windows acceptance checks.

## Work and AI

- [Agentic browsing](agentic-browsing.md): the browser-execution layer behind Work and how it is qualified.
- [Bounded browser session](agent-browser-session.md): the semantic locate/act controller session.
- Work runtime subsystems:
  [execution](agent-work-execution.md),
  [scoped runtime](agent-work-scoped-runtime.md),
  [composition](agent-work-composition.md),
  [native lifetimes](agent-work-lifetimes.md),
  [persistence](agent-work-persistence.md),
  [retained resources](agent-work-resources.md),
  [discovery](agent-work-discovery.md),
  [inspection](agent-work-inspection.md),
  [retained inspection evidence](agent-work-retained-inspection-evidence.md),
  [native actions](agent-work-native-actions.md),
  [retained actions](agent-work-retained-actions.md),
  [forms](agent-work-forms.md),
  [results](agent-work-results.md),
  [artifacts](agent-work-artifacts.md),
  [evidence](agent-work-evidence.md),
  [review](agent-work-review.md).
- [Evidence tooling](../eval/agentic-browsing/README.md): committed qualification manifests and the probes that produce them.

## Extensions

- [Using extensions](extensions-user-guide.md): installing from the Chrome Web Store and what to expect.
- [Initial release scope](extension-release-scope.md): what extension support covers and its completion criteria.
- [Windows extension QA](windows-extension-qa.md): the isolated QA build, acceptance checklist and performance checks.
- [Windows extension lab](../crates/zephium-webext-windows/README.md): native qualification harness.

## Ad blocking

- [Ad and tracker protection](adblock.md): the native network blocker, cosmetic hiding and release gates.
- [Cosmetic index delivery](adblock-cosmetic-index.md): keeping the generic cosmetic index out of every page.
- [Blocker fuzz harness](../crates/zephium-blocker/fuzz/README.md): running the fuzz targets.

## Contributing and maintenance

- [Contributing](../CONTRIBUTING.md): prerequisites, workflow and quality bar.
- [Security maintenance](security-maintenance.md): recurring security and engine-floor checks.
- [Release engineering](../.github/RELEASE.md): building, signing and publishing releases.
- [Frame guide](../frame/README.md): layout of the Svelte frontend.
