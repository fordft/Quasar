//! Commands as data: one registry feeds native menus, the palette, the
//! launcher, the keymap and Keyboard settings. Execution stays in the shell.

use std::collections::HashMap;

use crate::accelerator::{Accelerator, AcceleratorError, Platform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    App,
    File,
    Edit,
    View,
    History,
    Bookmarks,
    Window,
    Help,
    /// Keyboard only: never in a menu or the launcher, listed in Keyboard
    /// settings.
    Keys,
    /// Not placed in any menu; bound to a system-wide shortcut.
    Global,
    /// Work pane bindings: menu-hosted so page keystrokes reach them, enabled
    /// only while the pane is shown.
    Work,
}

/// Default keys per platform. Conventions differ (Show All History is
/// Command-Y on macOS but Ctrl+Y is Redo on Windows), so each side is
/// declared rather than derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys {
    pub mac: Option<&'static str>,
    pub other: Option<&'static str>,
}

impl Keys {
    pub const NONE: Keys = Keys {
        mac: None,
        other: None,
    };

    const fn same(keys: &'static str) -> Keys {
        Keys {
            mac: Some(keys),
            other: Some(keys),
        }
    }

    pub const fn on(self, platform: Platform) -> Option<&'static str> {
        match platform {
            Platform::Mac => self.mac,
            Platform::Other => self.other,
        }
    }
}

pub struct CommandSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub keys: Keys,
    pub group: Group,
}

impl CommandSpec {
    pub fn default_accelerator(&self) -> Option<&'static str> {
        self.keys.on(Platform::CURRENT)
    }

    /// Whether Keyboard settings may rebind it. The launcher owns its own
    /// recorder, and Work bindings are fixed to the pane.
    pub fn customizable(&self) -> bool {
        self.id != "launcher.toggle" && self.group != Group::Work
    }
}

const fn command(id: &'static str, title: &'static str, keys: Keys, group: Group) -> CommandSpec {
    CommandSpec {
        id,
        title,
        keys,
        group,
    }
}

const fn mac_only(keys: &'static str) -> Keys {
    Keys {
        mac: Some(keys),
        other: None,
    }
}

const fn split(mac: &'static str, other: &'static str) -> Keys {
    Keys {
        mac: Some(mac),
        other: Some(other),
    }
}

pub const REGISTRY: &[CommandSpec] = &[
    command(
        "browser.settings",
        "Settings…",
        Keys::same("CmdOrCtrl+,"),
        Group::App,
    ),
    command("tab.new", "New Tab", Keys::same("CmdOrCtrl+T"), Group::File),
    command(
        "window.newPrivate",
        "New Private Window",
        Keys::same("CmdOrCtrl+Shift+N"),
        Group::File,
    ),
    command(
        "window.closePrivate",
        "Close Private Window",
        Keys::NONE,
        Group::File,
    ),
    // Ctrl+Alt is AltGr on many Windows and Linux layouts, so only macOS
    // gets a default here.
    command(
        "note.new",
        "New Note",
        mac_only("CmdOrCtrl+Alt+N"),
        Group::File,
    ),
    command("split.choose", "Split View…", Keys::NONE, Group::File),
    command(
        "page.print",
        "Print…",
        Keys::same("CmdOrCtrl+P"),
        Group::File,
    ),
    command(
        "tab.close",
        "Close Tab",
        Keys::same("CmdOrCtrl+W"),
        Group::File,
    ),
    command("find.show", "Find…", Keys::same("CmdOrCtrl+F"), Group::Edit),
    command(
        "find.next",
        "Find Next",
        Keys::same("CmdOrCtrl+G"),
        Group::Edit,
    ),
    command(
        "find.previous",
        "Find Previous",
        Keys::same("CmdOrCtrl+Shift+G"),
        Group::Edit,
    ),
    command(
        "page.copyLink",
        "Copy Link",
        Keys::same("CmdOrCtrl+Shift+C"),
        Group::Edit,
    ),
    command(
        "sidebar.toggleCompact",
        "Compact Mode",
        Keys::same("CmdOrCtrl+Shift+S"),
        Group::View,
    ),
    // Hiding the sidebar needs somewhere for the window controls to live while
    // it is away; only macOS has them outside it.
    command(
        "sidebar.toggleAutoHide",
        "Auto-hide Sidebar",
        mac_only("CmdOrCtrl+S"),
        Group::View,
    ),
    command(
        "nav.reload",
        "Reload Page",
        Keys::same("CmdOrCtrl+R"),
        Group::View,
    ),
    command(
        "nav.stop",
        "Stop Loading",
        Keys::same("CmdOrCtrl+."),
        Group::View,
    ),
    command(
        "page.devtools",
        "Developer Tools",
        split("CmdOrCtrl+Alt+I", "Ctrl+Shift+I"),
        Group::View,
    ),
    command(
        "url.focus",
        "Open Location",
        Keys::same("CmdOrCtrl+L"),
        Group::View,
    ),
    command("zoom.in", "Zoom In", Keys::same("CmdOrCtrl+="), Group::View),
    command(
        "zoom.out",
        "Zoom Out",
        Keys::same("CmdOrCtrl+-"),
        Group::View,
    ),
    command(
        "zoom.reset",
        "Actual Size",
        Keys::same("CmdOrCtrl+0"),
        Group::View,
    ),
    command(
        "nav.back",
        "Back",
        Keys::same("CmdOrCtrl+["),
        Group::History,
    ),
    command(
        "nav.forward",
        "Forward",
        Keys::same("CmdOrCtrl+]"),
        Group::History,
    ),
    command(
        "tab.reopen",
        "Reopen Closed Tab",
        Keys::same("CmdOrCtrl+Shift+T"),
        Group::History,
    ),
    command(
        "browser.history",
        "Show All History",
        split("CmdOrCtrl+Shift+Y", "Ctrl+Shift+H"),
        Group::History,
    ),
    command(
        "bookmark.add",
        "Bookmark This Page",
        Keys::same("CmdOrCtrl+D"),
        Group::Bookmarks,
    ),
    command(
        "tool.bookmarks",
        "Show Bookmarks",
        split("CmdOrCtrl+Shift+B", "Ctrl+Shift+O"),
        Group::Bookmarks,
    ),
    command(
        "tab.next",
        "Next Tab",
        Keys::same("Ctrl+Tab"),
        Group::Window,
    ),
    command(
        "tab.previous",
        "Previous Tab",
        Keys::same("Ctrl+Shift+Tab"),
        Group::Window,
    ),
    // The same walk down the sidebar as Ctrl+Tab, on the arrows that point
    // along it. Ctrl+Alt stays clear of text: only letters and digits are
    // AltGr's.
    command(
        "tab.above",
        "Select Tab Above",
        Keys::same("CmdOrCtrl+Alt+Up"),
        Group::Window,
    ),
    command(
        "tab.below",
        "Select Tab Below",
        Keys::same("CmdOrCtrl+Alt+Down"),
        Group::Window,
    ),
    // The tool panels, each on Shift and its own letter, as Bookmarks is.
    // Never Alt on Windows: Ctrl+Alt is AltGr on many layouts. History takes
    // the browser's own key; its full page moves to Shift.
    command(
        "tool.notes",
        "Notes",
        split("CmdOrCtrl+Shift+E", "Ctrl+Shift+E"),
        Group::Window,
    ),
    command(
        "tool.ai",
        "Ask Quasar",
        Keys::same("CmdOrCtrl+E"),
        Group::Window,
    ),
    command(
        "tool.tasks",
        "Tasks",
        split("CmdOrCtrl+Shift+K", "Ctrl+Shift+K"),
        Group::Window,
    ),
    command(
        "tool.time",
        "Time",
        split("CmdOrCtrl+Shift+U", "Ctrl+Shift+F"),
        Group::Window,
    ),
    command(
        "tool.history",
        "History",
        split("CmdOrCtrl+Y", "Ctrl+H"),
        Group::Window,
    ),
    command(
        "tool.downloads",
        "Downloads",
        split("CmdOrCtrl+Shift+J", "Ctrl+J"),
        Group::Window,
    ),
    command("browser.tasks", "Show All Tasks", Keys::NONE, Group::Window),
    command("browser.notes", "Show All Notes", Keys::NONE, Group::Window),
    command("browser.time", "Show Time", Keys::NONE, Group::Window),
    command(
        "settings.shortcuts",
        "Keyboard Shortcuts",
        Keys::NONE,
        Group::Help,
    ),
    command(
        "tab.select.1",
        "Select Tab 1",
        Keys::same("CmdOrCtrl+1"),
        Group::Keys,
    ),
    command(
        "tab.select.2",
        "Select Tab 2",
        Keys::same("CmdOrCtrl+2"),
        Group::Keys,
    ),
    command(
        "tab.select.3",
        "Select Tab 3",
        Keys::same("CmdOrCtrl+3"),
        Group::Keys,
    ),
    command(
        "tab.select.4",
        "Select Tab 4",
        Keys::same("CmdOrCtrl+4"),
        Group::Keys,
    ),
    command(
        "tab.select.5",
        "Select Tab 5",
        Keys::same("CmdOrCtrl+5"),
        Group::Keys,
    ),
    command(
        "tab.select.6",
        "Select Tab 6",
        Keys::same("CmdOrCtrl+6"),
        Group::Keys,
    ),
    command(
        "tab.select.7",
        "Select Tab 7",
        Keys::same("CmdOrCtrl+7"),
        Group::Keys,
    ),
    command(
        "tab.select.8",
        "Select Tab 8",
        Keys::same("CmdOrCtrl+8"),
        Group::Keys,
    ),
    command(
        "tab.select.last",
        "Select Last Tab",
        Keys::same("CmdOrCtrl+9"),
        Group::Keys,
    ),
    command(
        "space.select.1",
        "Switch to Space 1",
        split("Ctrl+1", "Ctrl+Shift+1"),
        Group::Keys,
    ),
    command(
        "space.select.2",
        "Switch to Space 2",
        split("Ctrl+2", "Ctrl+Shift+2"),
        Group::Keys,
    ),
    command(
        "space.select.3",
        "Switch to Space 3",
        split("Ctrl+3", "Ctrl+Shift+3"),
        Group::Keys,
    ),
    command(
        "space.select.4",
        "Switch to Space 4",
        split("Ctrl+4", "Ctrl+Shift+4"),
        Group::Keys,
    ),
    command(
        "space.select.5",
        "Switch to Space 5",
        split("Ctrl+5", "Ctrl+Shift+5"),
        Group::Keys,
    ),
    command(
        "space.select.6",
        "Switch to Space 6",
        split("Ctrl+6", "Ctrl+Shift+6"),
        Group::Keys,
    ),
    command(
        "space.select.7",
        "Switch to Space 7",
        split("Ctrl+7", "Ctrl+Shift+7"),
        Group::Keys,
    ),
    command(
        "space.select.8",
        "Switch to Space 8",
        split("Ctrl+8", "Ctrl+Shift+8"),
        Group::Keys,
    ),
    command(
        "space.select.9",
        "Switch to Space 9",
        split("Ctrl+9", "Ctrl+Shift+9"),
        Group::Keys,
    ),
    command(
        "space.next",
        "Next Space",
        Keys::same("CmdOrCtrl+Alt+Right"),
        Group::Keys,
    ),
    command(
        "space.previous",
        "Previous Space",
        Keys::same("CmdOrCtrl+Alt+Left"),
        Group::Keys,
    ),
    command(
        "work.pane.close",
        "Close Pane",
        Keys::same("Escape"),
        Group::Work,
    ),
    command(
        "work.pane.openInBrowse",
        "Open Pane in Browse",
        Keys::same("CmdOrCtrl+Shift+Return"),
        Group::Work,
    ),
    // Starts and stops dictation in the Work bar; elsewhere it does nothing.
    command(
        "work.dictate",
        "Dictate",
        Keys::same("CmdOrCtrl+Shift+D"),
        Group::Keys,
    ),
    command(
        "launcher.toggle",
        "Toggle Launcher",
        Keys::same("CmdOrCtrl+Shift+Space"),
        Group::Global,
    ),
    command(
        "theme.system",
        "Appearance: System",
        Keys::NONE,
        Group::Global,
    ),
    command(
        "theme.light",
        "Appearance: Light",
        Keys::NONE,
        Group::Global,
    ),
    command("theme.dark", "Appearance: Dark", Keys::NONE, Group::Global),
];

/// Overrides one person may hold. Bounded so a stored keymap can never grow
/// past the registry it describes.
pub const MAX_OVERRIDES: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCommand {
    pub id: &'static str,
    pub title: &'static str,
    /// Canonical text for this platform, or None when unbound.
    pub accelerator: Option<String>,
    pub group: Group,
    pub customized: bool,
}

pub fn get(id: &str) -> Option<&'static CommandSpec> {
    REGISTRY.iter().find(|c| c.id == id)
}

/// Applies keymap overrides onto the defaults. An override with an empty
/// string removes the shortcut; unknown ids, fixed commands and overrides
/// that no longer validate fall back to the default.
/// The feature switch a command belongs to (`ai`, `work`, `notes`, `tasks`,
/// `time`), so a command whose feature is off is neither offered nor run.
/// Mirrored by `commandOffered` in the frame's preferences domain.
pub fn feature_of(id: &str) -> Option<&'static str> {
    match id {
        "work.dictate" | "tool.ai" | "mode.choose" => Some("ai"),
        "browser.work" | "mode.work" => Some("work"),
        "tool.notes" | "browser.notes" | "note.new" => Some("notes"),
        "tool.tasks" | "browser.tasks" => Some("tasks"),
        "tool.time" | "browser.time" | "settings.focus" => Some("time"),
        id if id.starts_with("work.") => Some("work"),
        id if id.starts_with("focus.") => Some("time"),
        _ => None,
    }
}

pub fn resolve(overrides: &HashMap<String, String>) -> Vec<ResolvedCommand> {
    resolve_on(overrides, Platform::CURRENT)
}

fn resolve_on(overrides: &HashMap<String, String>, platform: Platform) -> Vec<ResolvedCommand> {
    REGISTRY
        .iter()
        .map(|c| {
            let canonical =
                |text: &str| Accelerator::parse(text, platform).map(|a| a.format(platform));
            let custom =
                overrides
                    .get(c.id)
                    .filter(|_| c.customizable())
                    .and_then(|text| match text.as_str() {
                        "" => Some(None),
                        text => Accelerator::parse(text, platform)
                            .filter(|a| a.bindable(platform).is_ok())
                            .map(|a| Some(a.format(platform))),
                    });
            ResolvedCommand {
                id: c.id,
                title: c.title,
                accelerator: match &custom {
                    Some(chosen) => chosen.clone(),
                    None => c.keys.on(platform).and_then(canonical),
                },
                group: c.group,
                customized: custom.is_some(),
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeymapError {
    UnknownCommand,
    Fixed,
    Accelerator(AcceleratorError),
    /// Already bound to another command, named here.
    Conflict(&'static str),
}

/// Checks a new binding for `id` against the resolved keymap. `None` unbinds.
/// Returns the canonical text to store.
pub fn validate_binding(
    resolved: &[ResolvedCommand],
    id: &str,
    accelerator: Option<&str>,
) -> Result<Option<String>, KeymapError> {
    validate_binding_on(resolved, id, accelerator, Platform::CURRENT)
}

fn validate_binding_on(
    resolved: &[ResolvedCommand],
    id: &str,
    accelerator: Option<&str>,
    platform: Platform,
) -> Result<Option<String>, KeymapError> {
    let spec = get(id).ok_or(KeymapError::UnknownCommand)?;
    if !spec.customizable() {
        return Err(KeymapError::Fixed);
    }
    let Some(text) = accelerator else {
        return Ok(None);
    };
    let parsed = Accelerator::parse(text, platform)
        .ok_or(KeymapError::Accelerator(AcceleratorError::Invalid))?;
    parsed
        .bindable(platform)
        .map_err(KeymapError::Accelerator)?;
    let canonical = parsed.format(platform);
    if let Some(other) = resolved
        .iter()
        .find(|c| c.id != id && c.accelerator.as_deref() == Some(canonical.as_str()))
    {
        return Err(KeymapError::Conflict(other.id));
    }
    Ok(Some(canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLATFORMS: [Platform; 2] = [Platform::Mac, Platform::Other];

    #[test]
    fn ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for c in REGISTRY {
            assert!(seen.insert(c.id), "duplicate command id {}", c.id);
        }
    }

    #[test]
    fn defaults_parse_and_never_collide_on_either_platform() {
        for platform in PLATFORMS {
            let mut seen = HashMap::new();
            for c in REGISTRY {
                let Some(text) = c.keys.on(platform) else {
                    continue;
                };
                let parsed = Accelerator::parse(text, platform)
                    .unwrap_or_else(|| panic!("{} default {text} parses", c.id));
                // Work pane keys are bare on purpose and enabled only while
                // the pane is shown; every other default meets the user rule.
                if c.group != Group::Work {
                    assert_eq!(
                        parsed.bindable(platform),
                        Ok(()),
                        "{} default {text} on {platform:?}",
                        c.id
                    );
                }
                if let Some(other) = seen.insert(parsed, c.id) {
                    panic!("{} and {} share {text} on {platform:?}", other, c.id);
                }
            }
        }
    }

    #[test]
    fn overrides_replace_and_remove_shortcuts() {
        let mut overrides = HashMap::new();
        overrides.insert("tab.new".into(), "CmdOrCtrl+N".into());
        overrides.insert("tab.close".into(), "".into());
        overrides.insert("bogus.id".into(), "CmdOrCtrl+X".into());

        let resolved = resolve_on(&overrides, Platform::Mac);
        let find = |id: &str| resolved.iter().find(|c| c.id == id).unwrap();
        assert_eq!(find("tab.new").accelerator.as_deref(), Some("Cmd+N"));
        assert!(find("tab.new").customized);
        assert_eq!(find("tab.close").accelerator, None);
        assert!(find("tab.close").customized);
        assert_eq!(find("nav.reload").accelerator.as_deref(), Some("Cmd+R"));
        assert!(!find("nav.reload").customized);
        assert!(!resolved.iter().any(|c| c.id == "bogus.id"));
    }

    #[test]
    fn stale_or_forbidden_overrides_fall_back_to_the_default() {
        let mut overrides = HashMap::new();
        overrides.insert("tab.new".into(), "Cmd+Q".into());
        overrides.insert("nav.reload".into(), "Hyper+R".into());
        overrides.insert("work.pane.close".into(), "Cmd+K".into());
        let resolved = resolve_on(&overrides, Platform::Mac);
        let find = |id: &str| resolved.iter().find(|c| c.id == id).unwrap();
        assert_eq!(find("tab.new").accelerator.as_deref(), Some("Cmd+T"));
        assert_eq!(find("nav.reload").accelerator.as_deref(), Some("Cmd+R"));
        assert_eq!(
            find("work.pane.close").accelerator.as_deref(),
            Some("Escape")
        );
    }

    #[test]
    fn a_binding_names_the_command_it_would_steal_from() {
        let resolved = resolve_on(&HashMap::new(), Platform::Other);
        assert_eq!(
            validate_binding_on(&resolved, "tab.new", Some("Ctrl+W"), Platform::Other),
            Err(KeymapError::Conflict("tab.close"))
        );
        assert_eq!(
            validate_binding_on(&resolved, "tab.new", Some("CmdOrCtrl+T"), Platform::Other),
            Ok(Some("Ctrl+T".into()))
        );
        assert_eq!(
            validate_binding_on(&resolved, "tab.new", Some("Ctrl+V"), Platform::Other),
            Err(KeymapError::Accelerator(AcceleratorError::Reserved))
        );
        assert_eq!(
            validate_binding_on(&resolved, "work.pane.close", None, Platform::Other),
            Err(KeymapError::Fixed)
        );
        assert_eq!(
            validate_binding_on(&resolved, "nope", None, Platform::Other),
            Err(KeymapError::UnknownCommand)
        );
        assert_eq!(
            validate_binding_on(&resolved, "tab.new", None, Platform::Other),
            Ok(None)
        );
    }

    #[test]
    fn history_follows_each_platform_convention() {
        // The platform's history key opens the quick panel; the full page is
        // the same key with Shift.
        let history = get("tool.history").expect("history panel command");
        assert_eq!(history.keys.on(Platform::Mac), Some("CmdOrCtrl+Y"));
        assert_eq!(history.keys.on(Platform::Other), Some("Ctrl+H"));
        let page = get("browser.history").expect("history page command");
        assert_eq!(page.keys.on(Platform::Mac), Some("CmdOrCtrl+Shift+Y"));
        assert_eq!(page.keys.on(Platform::Other), Some("Ctrl+Shift+H"));
    }

    #[test]
    fn ask_panel_takes_the_bare_e_and_stays_rebindable() {
        let ask = get("tool.ai").expect("ask command");
        assert_eq!(ask.keys.on(Platform::Mac), Some("CmdOrCtrl+E"));
        assert_eq!(ask.keys.on(Platform::Other), Some("CmdOrCtrl+E"));
        assert!(ask.customizable());
    }

    #[test]
    fn compact_sidebar_is_a_registered_view_command() {
        let command = get("sidebar.toggleCompact").expect("compact sidebar command");
        assert_eq!(command.title, "Compact Mode");
        assert_eq!(command.group, Group::View);
    }

    #[test]
    fn auto_hide_is_a_mac_only_view_command_on_the_arc_key() {
        let command = get("sidebar.toggleAutoHide").expect("auto-hide sidebar command");
        assert_eq!(command.title, "Auto-hide Sidebar");
        assert_eq!(command.group, Group::View);
        assert_eq!(command.keys.on(Platform::Mac), Some("CmdOrCtrl+S"));
        assert_eq!(command.keys.on(Platform::Other), None);
    }

    #[test]
    fn split_selection_is_a_registered_ui_command_without_an_accelerator() {
        let command = get("split.choose").expect("split selection command");
        assert_eq!(command.title, "Split View…");
        assert_eq!(command.keys, Keys::NONE);
        assert_eq!(command.group, Group::File);
    }

    #[test]
    fn every_feature_command_names_its_switch() {
        assert_eq!(feature_of("work.dictate"), Some("ai"));
        assert_eq!(feature_of("tool.ai"), Some("ai"));
        assert_eq!(feature_of("work.pane.close"), Some("work"));
        assert_eq!(feature_of("browser.work"), Some("work"));
        for id in ["tool.notes", "browser.notes", "note.new"] {
            assert_eq!(feature_of(id), Some("notes"));
        }
        for id in ["tool.tasks", "browser.tasks"] {
            assert_eq!(feature_of(id), Some("tasks"));
        }
        for id in ["tool.time", "browser.time"] {
            assert_eq!(feature_of(id), Some("time"));
        }
        assert_eq!(feature_of("tab.new"), None);
        assert_eq!(feature_of("tool.bookmarks"), None);
    }
}
