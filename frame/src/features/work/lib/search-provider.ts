import * as m from "$shared/i18n/messages";

const VENDORS: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  google: "Google",
  zephium: "Quasar",
  openrouter: "OpenRouter",
};

const ENGINES: Record<string, string> = {
  google: "Google",
  duckduckgo: "DuckDuckGo",
  bing: "Bing",
  brave: "Brave Search",
};

/**
 * Who supplied a cited page, as the evidence names it: OpenAI's own search,
 * the chosen model's provider searching natively (its catalog id says
 * which, `anthropic/claude-…`), or the person's search engine's results page.
 */
export function searchProvider(
  provider: string,
  model: string,
): { provider: string; model: string } {
  if (provider === "browser")
    return {
      provider: ENGINES[model] ?? m.work_provider_your_engine(),
      model: m.work_provider_results_page(),
    };
  const [vendor, ...rest] = model.split("/");
  const name = rest.length ? rest.join("/") : model;
  if (provider === "open_ai")
    return { provider: m.work_provider_web_search({ provider: "OpenAI" }), model: name };
  const known = vendor && rest.length ? VENDORS[vendor] : undefined;
  return { provider: known ?? m.work_provider_model(), model: name };
}
