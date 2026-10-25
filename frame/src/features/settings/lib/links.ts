import * as m from "$shared/i18n/messages";
import { IS_MAC } from "$shared/platform";

export const WEBSITE = "https://github.com/fordft/Quasar";

export const links = [
  { id: "website", url: WEBSITE, label: () => m.about_link_website() },
  // Without a menu bar there is no Help menu, so the documentation it leads
  // to is named Help here, where Windows and Linux look for it.
  {
    id: "docs",
    url: `${WEBSITE}/tree/main/docs`,
    label: () => (IS_MAC ? m.about_link_docs() : m.native_help()),
  },
  { id: "github", url: WEBSITE, label: () => "GitHub" },
] as const;
