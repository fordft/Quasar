//! Spaces from privileged chrome, the menu bar and the tab menu, and the
//! window ground a space's own theme paints.
use std::sync::Mutex;

use tauri::menu::{
    CheckMenuItemBuilder, IsMenuItem, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu,
    SubmenuBuilder,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::menu_text;
use zephium_app::{Command, Handle, SpaceCommand};
use zephium_core::ids::{ItemId, SpaceId};
use zephium_core::spaces::{SpaceIcon, SpaceLook, SpaceTheme};
use zephium_ipc::{ItemsState, OperationAdmission, SpaceIconView, SpaceThemeView};

const SWITCH: &str = "space.switch=";
const MOVE_ACTIVE: &str = "space.moveActive=";
/// Handled by chrome, which opens its switcher to make or edit a space.
const NEW: &str = "space.new";
const EDIT: &str = "space.edit";
pub(crate) const TAB_MENU_MOVE: &str = "space=";

#[derive(Clone, Debug, serde::Deserialize, specta::Type)]
pub(crate) struct SpaceLookInputV1 {
    icon: Option<SpaceIconView>,
    color: u8,
    theme: Option<SpaceThemeView>,
}

impl SpaceLookInputV1 {
    fn look(self) -> Option<SpaceLook> {
        let look = SpaceLook {
            icon: self.icon.map(|icon| match icon {
                SpaceIconView::Symbol { index } => SpaceIcon::Symbol(index),
                SpaceIconView::Emoji { text } => SpaceIcon::Emoji(text),
            }),
            color: self.color,
            theme: self.theme.map(|theme| match theme {
                SpaceThemeView::Preset { id } => SpaceTheme::Preset(id),
                SpaceThemeView::Custom { seeds } => SpaceTheme::Custom(seeds),
            }),
        };
        look.valid().then_some(look)
    }
}

#[derive(Clone, Default, PartialEq)]
struct Shown {
    /// The focused profile's spaces, in order, with their names.
    spaces: Vec<(String, String)>,
    active: Option<String>,
    private: bool,
}

impl Shown {
    /// Whether the menus need rebuilding rather than updating: which spaces
    /// there are, or whether the window can use them at all, changed.
    fn reshaped_by(&self, next: &Shown) -> bool {
        self.private != next.private
            || self.spaces.len() != next.spaces.len()
            || self
                .spaces
                .iter()
                .zip(&next.spaces)
                .any(|(was, now)| was.0 != now.0)
    }
}

#[derive(Default)]
struct Followed {
    shown: Shown,
    /// The active space's theme as `ui.theme` and `ui.theme-custom` values.
    theme: Option<(String, Option<String>)>,
}

static FOLLOWED: Mutex<Option<Followed>> = Mutex::new(None);

fn shown() -> Shown {
    FOLLOWED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|followed| followed.shown.clone())
        .unwrap_or_default()
}

/// The theme native paints while a themed space is active.
pub(crate) fn theme() -> Option<(String, Option<String>)> {
    FOLLOWED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|followed| followed.theme.clone())
}

/// Follows each sidebar projection: the menus list the spaces it names, and
/// the window ground takes on the active space's theme.
pub(crate) fn follow(app: &AppHandle, state: &ItemsState) {
    let active = state
        .active_space_id
        .as_ref()
        .and_then(|id| state.spaces.iter().find(|space| &space.id == id));
    let next = Shown {
        spaces: state
            .spaces
            .iter()
            .map(|space| (space.id.clone(), space.name.clone()))
            .collect(),
        active: state.active_space_id.clone(),
        private: state
            .profile
            .as_ref()
            .is_some_and(|profile| profile.kind == zephium_ipc::ProfileKindView::Incognito),
    };
    let theme = active
        .and_then(|space| space.theme.as_ref())
        .map(|theme| match theme {
            SpaceThemeView::Preset { id } => (id.clone(), None),
            SpaceThemeView::Custom { seeds } => {
                (zephium_core::theme::CUSTOM.to_owned(), Some(seeds.clone()))
            }
        });
    let (menus, ground) = {
        let mut followed = FOLLOWED.lock().unwrap_or_else(|e| e.into_inner());
        let followed = followed.get_or_insert_with(Followed::default);
        let menus = followed.shown != next;
        let membership = followed.shown.reshaped_by(&next);
        let ground = followed.theme != theme;
        followed.shown = next;
        followed.theme = theme;
        (menus.then_some(membership), ground)
    };
    if ground {
        super::reinstall_native_materials(app);
    }
    // Switching only moves the check: the menu bar is never set again for it,
    // since setting it makes every title flash.
    #[cfg(target_os = "macos")]
    if let Some(membership) = menus {
        let handle = app.clone();
        let queued = app.run_on_main_thread(move || {
            let Some(keymap) = handle.try_state::<super::keymap::Keymap>() else {
                return;
            };
            if !membership && sync_menu_bar(&handle).is_ok_and(|synced| synced) {
                return;
            }
            if super::install_menu_bar(&handle, &keymap.overrides(), keymap.work_pane_shown())
                .is_err()
            {
                super::write_diagnostic(format_args!("menu: spaces were not applied"));
            }
        });
        if queued.is_err() {
            super::write_diagnostic(format_args!("menu: spaces were not applied"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = menus;
}

/// Brings the installed Spaces menu up to date in place: the check, the names,
/// and the spaces a tab can move to. False when the menu is not the shape the
/// spaces describe, which asks for a rebuild instead. Main thread only.
#[cfg(target_os = "macos")]
fn sync_menu_bar(app: &AppHandle) -> tauri::Result<bool> {
    use tauri::menu::MenuItemKind;

    let Some(menu) = app.menu() else {
        return Ok(false);
    };
    let shown = shown();
    for entry in menu.items()? {
        let MenuItemKind::Submenu(section) = entry else {
            continue;
        };
        let items = section.items()?;
        if !items
            .iter()
            .any(|item| matches!(item, MenuItemKind::MenuItem(item) if item.id() == NEW))
        {
            continue;
        }
        let mut checks = 0;
        let mut moved = false;
        for item in &items {
            match item {
                MenuItemKind::Check(check) => {
                    let Some((id, name)) = shown.spaces.get(checks) else {
                        return Ok(false);
                    };
                    check.set_checked(shown.active.as_deref() == Some(id.as_str()))?;
                    check.set_text(super::extension_menu_label(name))?;
                    checks += 1;
                }
                MenuItemKind::Submenu(moves) => {
                    while moves.remove_at(0)?.is_some() {}
                    for item in move_items(app, MOVE_ACTIVE, &shown)? {
                        moves.append(item.as_ref())?;
                    }
                    moved = true;
                }
                _ => {}
            }
        }
        return Ok(checks == shown.spaces.len() && moved);
    }
    Ok(false)
}

fn space_of(value: &str) -> Option<SpaceId> {
    SpaceId::parse(value).filter(|id| id.to_string() == value)
}

fn admitted(caller: &WebviewWindow, command: &str) -> bool {
    super::authorize(caller, super::CallerPolicy::Main, command)
        && !super::shutdown_started(caller.app_handle())
}

fn dispatch(app: &AppHandle, shell: &Handle, command: SpaceCommand) -> OperationAdmission {
    super::dispatch_operation(app, shell, Command::Space(Box::new(command)))
}

#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_create(
    caller: WebviewWindow,
    shell: State<'_, Handle>,
    name: String,
    look: SpaceLookInputV1,
) -> OperationAdmission {
    if !admitted(&caller, "spaces_create") || !super::bounded(&name, super::MAX_PROFILE_NAME_BYTES)
    {
        return super::rejected_operation();
    }
    let Some(look) = look.look() else {
        return super::rejected_operation();
    };
    dispatch(
        caller.app_handle(),
        &shell,
        SpaceCommand::Create { name, look },
    )
}

#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_update(
    caller: WebviewWindow,
    shell: State<'_, Handle>,
    space: String,
    name: String,
    look: SpaceLookInputV1,
) -> OperationAdmission {
    let (Some(id), Some(look)) = (space_of(&space), look.look()) else {
        return super::rejected_operation();
    };
    if !admitted(&caller, "spaces_update") || !super::bounded(&name, super::MAX_PROFILE_NAME_BYTES)
    {
        return super::rejected_operation();
    }
    dispatch(
        caller.app_handle(),
        &shell,
        SpaceCommand::Update { id, name, look },
    )
}

#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_delete(
    caller: WebviewWindow,
    shell: State<'_, Handle>,
    space: String,
) -> OperationAdmission {
    match space_of(&space).filter(|_| admitted(&caller, "spaces_delete")) {
        Some(id) => dispatch(caller.app_handle(), &shell, SpaceCommand::Delete(id)),
        None => super::rejected_operation(),
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_reorder(
    caller: WebviewWindow,
    shell: State<'_, Handle>,
    space: String,
    before: Option<String>,
) -> OperationAdmission {
    let before = match before.as_deref().map(space_of) {
        None => None,
        Some(Some(id)) => Some(id),
        Some(None) => return super::rejected_operation(),
    };
    match space_of(&space).filter(|_| admitted(&caller, "spaces_reorder")) {
        Some(id) => dispatch(
            caller.app_handle(),
            &shell,
            SpaceCommand::Reorder { id, before },
        ),
        None => super::rejected_operation(),
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_switch(
    caller: WebviewWindow,
    shell: State<'_, Handle>,
    space: String,
) -> OperationAdmission {
    match space_of(&space).filter(|_| admitted(&caller, "spaces_switch")) {
        Some(id) => dispatch(caller.app_handle(), &shell, SpaceCommand::Switch(id)),
        None => super::rejected_operation(),
    }
}

/// The next (1) or previous (-1) space, for a swipe over the tab list.
#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_step(
    caller: WebviewWindow,
    shell: State<'_, Handle>,
    step: i8,
) -> OperationAdmission {
    if !admitted(&caller, "spaces_step") || !matches!(step, -1 | 1) {
        return super::rejected_operation();
    }
    dispatch(caller.app_handle(), &shell, SpaceCommand::Step(step))
}

#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_move_tab(
    caller: WebviewWindow,
    shell: State<'_, Handle>,
    tab: String,
    space: String,
) -> OperationAdmission {
    let tab = ItemId::parse(&tab).filter(|_| tab.len() <= super::MAX_ITEM_ID_BYTES);
    match (tab, space_of(&space)) {
        (Some(tab), Some(space)) if admitted(&caller, "spaces_move_tab") => dispatch(
            caller.app_handle(),
            &shell,
            SpaceCommand::MoveTab { tab, space },
        ),
        _ => super::rejected_operation(),
    }
}

/// The switcher at rail width, where the column has no room for its own.
#[tauri::command]
#[specta::specta]
pub(crate) fn spaces_menu_popup(caller: WebviewWindow, app: AppHandle, x: f64, y: f64) -> bool {
    if !super::authorize(&caller, super::CallerPolicy::Main, "spaces_menu_popup") {
        return false;
    }
    let (Ok(size), Ok(scale)) = (caller.inner_size(), caller.scale_factor()) else {
        return false;
    };
    if !scale.is_finite() || scale <= 0.0 {
        return false;
    }
    let Some(anchor) = super::menu_popup_anchor(
        x,
        y,
        f64::from(size.width) / scale,
        f64::from(size.height) / scale,
    ) else {
        return false;
    };
    let overrides = super::keymap_overrides(&app);
    let Ok(menu) = build_popup(&app, &overrides) else {
        return false;
    };
    caller.popup_menu_at(&menu, anchor).is_ok()
}

/// Menu ids this module owns; anything else is not a space action.
pub(crate) fn execute(app: &AppHandle, id: &str) -> Option<OperationAdmission> {
    let shell = || app.try_state::<Handle>();
    if let Some(space) = id.strip_prefix(SWITCH) {
        let (Some(shell), Some(space)) = (shell(), space_of(space)) else {
            return Some(super::rejected_operation());
        };
        return Some(dispatch(app, &shell, SpaceCommand::Switch(space)));
    }
    if let Some(space) = id.strip_prefix(MOVE_ACTIVE) {
        let (Some(shell), Some(space)) = (shell(), space_of(space)) else {
            return Some(super::rejected_operation());
        };
        return Some(dispatch(app, &shell, SpaceCommand::MoveActive(space)));
    }
    if matches!(id, NEW | EDIT) {
        return Some(
            if super::try_emit_to_privileged(app, super::MAIN_LABEL, super::EVENT_UI, &id) {
                super::accepted_ui_operation()
            } else {
                super::rejected_operation()
            },
        );
    }
    None
}

/// A tab menu action naming the space to move its tab to.
pub(crate) fn move_tab(app: &AppHandle, tab: ItemId, space: &str) -> OperationAdmission {
    match (app.try_state::<Handle>(), space_of(space)) {
        (Some(shell), Some(space)) => dispatch(app, &shell, SpaceCommand::MoveTab { tab, space }),
        _ => super::rejected_operation(),
    }
}

type Items = Vec<Box<dyn IsMenuItem<tauri::Wry>>>;

/// The spaces with a check on the active one, each on its key.
fn space_items(
    handle: &AppHandle,
    overrides: &std::collections::HashMap<String, String>,
    shown: &Shown,
) -> tauri::Result<Items> {
    let resolved = zephium_core::commands::resolve(overrides);
    let mut items: Items = Vec::new();
    for (index, (id, name)) in shown.spaces.iter().enumerate() {
        let command = format!("space.select.{}", index + 1);
        let key = resolved
            .iter()
            .find(|entry| entry.id == command)
            .and_then(|entry| {
                super::menu_accelerator(handle, entry.id, entry.accelerator.as_deref())
            })
            .map(str::to_owned);
        let id = if key.is_some() {
            command
        } else {
            format!("{SWITCH}{id}")
        };
        let mut builder = CheckMenuItemBuilder::with_id(id, super::extension_menu_label(name))
            .checked(shown.active.as_deref() == Some(&*shown.spaces[index].0));
        if let Some(key) = key {
            builder = builder.accelerator(key);
        }
        items.push(Box::new(builder.build(handle)?));
    }
    Ok(items)
}

fn move_items(handle: &AppHandle, prefix: &str, shown: &Shown) -> tauri::Result<Items> {
    let mut items: Items = Vec::new();
    for (id, name) in &shown.spaces {
        if shown.active.as_deref() == Some(id.as_str()) {
            continue;
        }
        items.push(Box::new(
            MenuItemBuilder::with_id(format!("{prefix}{id}"), super::extension_menu_label(name))
                .build(handle)?,
        ));
    }
    Ok(items)
}

fn refs(items: &Items) -> Vec<&dyn IsMenuItem<tauri::Wry>> {
    items.iter().map(|item| item.as_ref()).collect()
}

/// The menu bar's Spaces menu.
pub(crate) fn build_menu_section(
    handle: &AppHandle,
    overrides: &std::collections::HashMap<String, String>,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let shown = shown();
    let resolved = zephium_core::commands::resolve(overrides);
    let item = |id: &str| super::build_command_menu_item(handle, &resolved, id);
    let enabled = !shown.private && !shown.spaces.is_empty();
    let spaces = space_items(handle, overrides, &shown)?;
    let moves = move_items(handle, MOVE_ACTIVE, &shown)?;
    let move_to = SubmenuBuilder::new(handle, menu_text::label("Move Tab to Space"))
        .items(&refs(&moves))
        .enabled(enabled && !moves.is_empty())
        .build()?;
    SubmenuBuilder::new(handle, menu_text::label("Spaces"))
        .items(&refs(&spaces))
        .separator()
        .item(&item("space.next")?)
        .item(&item("space.previous")?)
        .separator()
        .item(
            &MenuItemBuilder::with_id(NEW, menu_text::label("New Space…"))
                .enabled(enabled)
                .build(handle)?,
        )
        .item(
            &MenuItemBuilder::with_id(EDIT, menu_text::label("Edit Space…"))
                .enabled(enabled)
                .build(handle)?,
        )
        .item(&move_to)
        .build()
}

fn build_popup(
    handle: &AppHandle,
    overrides: &std::collections::HashMap<String, String>,
) -> tauri::Result<Menu<tauri::Wry>> {
    let shown = shown();
    let mut items = space_items(handle, overrides, &shown)?;
    items.push(Box::new(PredefinedMenuItem::separator(handle)?));
    items.push(Box::new(
        MenuItemBuilder::with_id(NEW, menu_text::label("New Space…")).build(handle)?,
    ));
    items.push(Box::new(
        MenuItemBuilder::with_id(EDIT, menu_text::label("Edit Space…")).build(handle)?,
    ));
    Menu::with_items(handle, &refs(&items))
}

/// "Move to Space ▸" for the tab menu, or nothing when there is nowhere to go.
pub(crate) fn tab_menu_section(
    handle: &AppHandle,
    prefix: &str,
) -> tauri::Result<Option<Submenu<tauri::Wry>>> {
    let shown = shown();
    if shown.private {
        return Ok(None);
    }
    let moves = move_items(handle, prefix, &shown)?;
    if moves.is_empty() {
        return Ok(None);
    }
    SubmenuBuilder::new(handle, menu_text::label("Move to Space"))
        .items(&refs(&moves))
        .build()
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_from_chrome_is_admitted_only_when_it_validates() {
        let input = |color, theme: Option<SpaceThemeView>| SpaceLookInputV1 {
            icon: Some(SpaceIconView::Emoji {
                text: "🌿".into()
            }),
            color,
            theme,
        };
        let look = input(3, Some(SpaceThemeView::Preset { id: "nord".into() }))
            .look()
            .unwrap();
        assert_eq!(look.icon, Some(SpaceIcon::Emoji("🌿".into())));
        assert_eq!(look.theme, Some(SpaceTheme::Preset("nord".into())));
        assert!(input(10, None).look().is_none());
        let bare = SpaceLookInputV1 {
            icon: None,
            color: 2,
            theme: None,
        };
        assert_eq!(bare.look().map(|look| look.icon), Some(None));
        assert!(
            input(0, Some(SpaceThemeView::Custom { seeds: "{}".into() }))
                .look()
                .is_none()
        );
    }

    #[test]
    fn switching_or_renaming_never_reshapes_the_menus() {
        let shown = |active: &str, names: [&str; 2]| Shown {
            spaces: vec![("a".into(), names[0].into()), ("b".into(), names[1].into())],
            active: Some(active.into()),
            private: false,
        };
        let before = shown("a", ["Home", "Work"]);
        assert!(!before.reshaped_by(&shown("b", ["Home", "Work"])));
        assert!(!before.reshaped_by(&shown("a", ["Home", "Studio"])));
        let mut added = shown("a", ["Home", "Work"]);
        added.spaces.push(("c".into(), "New".into()));
        assert!(before.reshaped_by(&added));
        let mut private = shown("a", ["Home", "Work"]);
        private.private = true;
        assert!(before.reshaped_by(&private));
    }

    #[test]
    fn only_canonical_space_ids_are_accepted() {
        let id = SpaceId::generate().to_string();
        assert!(space_of(&id).is_some());
        assert!(space_of("nope").is_none());
    }
}
