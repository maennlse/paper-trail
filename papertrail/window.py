# SPDX-License-Identifier: GPL-3.0-or-later

from __future__ import annotations

from pathlib import Path

import gi

gi.require_version("Adw", "1")
gi.require_version("Gtk", "4.0")
gi.require_version("GtkSource", "5")

from gi.repository import Adw, Gdk, Gio, GLib, Gtk, GtkSource, Pango

from .folder_colors import PRESET_FOLDER_COLORS, custom_folder_colors, folder_color_css_class, is_custom_folder_color
from .folder_row import FolderRow
from .note_repository import NoteRecord, NoteRepository
from .note_row import NoteRow
from .settings import Settings


UI_DIR = Path(__file__).resolve().parent / "ui"
STYLES_DIR = Path(__file__).resolve().parent / "styles"
FIXED_SIDEBAR_WIDTH = 340
ALL_FOLDERS_PATH = Path("/__papertrail_all__")
SCHEME_PREVIEW_TEXT = """# Markdown
1. Numbered Lists
* Unnumbered and [Links](https://gnome.org)
* Preformatted Text
_Emphasis_ or *Emphasis* **Combined**
> Block quotes too!
"""
LIGHT_STYLE_SCHEME_CHOICES = [
    ("Adwaita", "Adwaita"),
    ("Builder", "builder"),
    ("Classic", "classic"),
    ("Cobalt", "cobalt-light"),
    ("Kate", "kate"),
    ("Peninsula", "peninsula"),
    ("Solarized", "solarized-light"),
    ("Tango", "tango"),
]
DARK_STYLE_SCHEME_CHOICES = [
    ("Adwaita", "Adwaita"),
    ("Builder", "builder"),
    ("Classic", "classic"),
    ("Cobalt", "cobalt-light"),
    ("Kate", "kate"),
    ("Peninsula", "peninsula"),
    ("Solarized", "solarized-light"),
    ("Oblivion", "oblivion"),
    ("Jollpi", "jollpi"),
]
STYLE_SCHEME_ALIASES = {
    "cobalt": "cobalt-light",
    "solarized": "solarized-light",
}


@Gtk.Template(filename=str(UI_DIR / "window.ui"))
class PaperTrailWindow(Adw.ApplicationWindow):
    __gtype_name__ = "PaperTrailWindow"

    main_paned: Gtk.Box = Gtk.Template.Child()
    sidebar_panel: Gtk.Box = Gtk.Template.Child()
    sidebar_divider: Gtk.Separator = Gtk.Template.Child()
    sidebar_search_button: Gtk.Button = Gtk.Template.Child()
    sidebar_search_revealer: Gtk.Revealer = Gtk.Template.Child()
    sidebar_folders_scroller: Gtk.ScrolledWindow = Gtk.Template.Child()
    sidebar_folders_viewport: Gtk.Viewport = Gtk.Template.Child()
    sidebar_add_folder_button: Gtk.Button = Gtk.Template.Child()
    sidebar_inline_add_box: Gtk.Box = Gtk.Template.Child()
    sidebar_inline_add_folder_button: Gtk.Button = Gtk.Template.Child()
    sidebar_folder_add_box: Gtk.Box = Gtk.Template.Child()
    folder_list: Gtk.Box = Gtk.Template.Child()
    note_list: Gtk.Box = Gtk.Template.Child()
    sidebar_search_entry: Gtk.SearchEntry = Gtk.Template.Child()
    sidebar_title_label: Gtk.Label = Gtk.Template.Child()
    sidebar_subtitle_label: Gtk.Label = Gtk.Template.Child()
    editor_stack: Gtk.Stack = Gtk.Template.Child()
    empty_page: Gtk.ScrolledWindow = Gtk.Template.Child()
    empty_state_stack: Gtk.Stack = Gtk.Template.Child()
    recent_notes_page: Gtk.Box = Gtk.Template.Child()
    empty_folder_page: Gtk.Box = Gtk.Template.Child()
    recent_notes_subtitle_label: Gtk.Label = Gtk.Template.Child()
    recent_notes_grid: Gtk.FlowBox = Gtk.Template.Child()
    editor_page: Gtk.ScrolledWindow = Gtk.Template.Child()
    editor_title_label: Gtk.Label = Gtk.Template.Child()
    editor_filename_label: Gtk.Label = Gtk.Template.Child()
    info_button: Gtk.ToggleButton = Gtk.Template.Child()
    info_revealer: Gtk.Revealer = Gtk.Template.Child()
    info_title_row: Adw.ActionRow = Gtk.Template.Child()
    info_name_row: Adw.ActionRow = Gtk.Template.Child()
    info_location_row: Adw.ActionRow = Gtk.Template.Child()
    info_type_row: Adw.ActionRow = Gtk.Template.Child()
    language_popout_button: Gtk.MenuButton = Gtk.Template.Child()
    language_popover: Gtk.Popover = Gtk.Template.Child()
    language_search_entry: Gtk.SearchEntry = Gtk.Template.Child()
    language_listbox: Gtk.ListBox = Gtk.Template.Child()
    editor_search_revealer: Gtk.Revealer = Gtk.Template.Child()
    editor_replace_revealer: Gtk.Revealer = Gtk.Template.Child()
    editor_search_entry: Gtk.SearchEntry = Gtk.Template.Child()
    editor_replace_entry: Gtk.Entry = Gtk.Template.Child()
    editor_replace_toggle_button: Gtk.ToggleButton = Gtk.Template.Child()
    editor_search_status_label: Gtk.Label = Gtk.Template.Child()
    info_delete_button: Gtk.Button = Gtk.Template.Child()
    notes_menu_button: Gtk.MenuButton = Gtk.Template.Child()

    def __init__(self, **kwargs) -> None:
        super().__init__(**kwargs)
        self.add_css_class("papertrail-window")
        self.language_popover.add_css_class("papertrail-popover")

        self.settings = Settings()
        self.repository = NoteRepository(self.settings.notes_dir)
        self.current_note: NoteRecord | None = None
        self._notes: list[NoteRecord] = []
        self._save_source_id: int | None = None
        self._loading_note = False
        self._sidebar_visible = True
        self._show_all_folders = False
        self._syncing_language_list = False
        self._syncing_info_fields = False
        self._syncing_preferences = False
        self._search_settings: GtkSource.SearchSettings | None = None
        self._search_context: GtkSource.SearchContext | None = None
        self._editor_zoom_provider = Gtk.CssProvider()
        self._folder_color_provider = Gtk.CssProvider()
        self._scheme_recolor_provider = Gtk.CssProvider()
        self._preferences_window: Adw.PreferencesDialog | None = None
        self._shortcuts_window: Adw.Dialog | None = None
        self._scheme_preview_buttons: dict[str, Gtk.ToggleButton] = {}
        self._scheme_preview_frames: dict[str, Gtk.Frame] = {}
        self._scheme_preview_checks: dict[str, Gtk.Image] = {}
        self._menu_theme_follow: Gtk.CheckButton | None = None
        self._menu_theme_light: Gtk.CheckButton | None = None
        self._menu_theme_dark: Gtk.CheckButton | None = None
        self._menu_zoom_button: Gtk.Button | None = None
        self._menu_zoom_label: Gtk.Label | None = None
        self._editor_context_popover: Gtk.PopoverMenu | None = None
        self._editor_context_actions: Gio.SimpleActionGroup | None = None
        self._editor_context_anchor = Gdk.Rectangle()
        self._editor_emoji_chooser: Gtk.EmojiChooser | None = None
        self._editor_emoji_anchor = Gdk.Rectangle()
        self._active_note_row: NoteRow | None = None
        self._active_folder_row: FolderRow | None = None
        self._folder_edit_window: Adw.Window | None = None
        self._folder_add_visibility_source_id: int | None = None
        self._preferences_size_sync_source_id: int | None = None

        self._install_css()
        self._refresh_folder_color_css()
        self._setup_editor()
        self._setup_actions()
        self._setup_menu()
        self._setup_language_chooser()

        self.set_default_size(self.settings.data.width, self.settings.data.height)
        self.sidebar_panel.set_size_request(FIXED_SIDEBAR_WIDTH, -1)
        self.sidebar_panel.set_hexpand(False)
        self.sidebar_panel.set_halign(Gtk.Align.START)
        self.sidebar_title_label.set_text("Paper Trail")
        self.sidebar_search_button.connect("clicked", self._on_sidebar_search_button_clicked)
        self.sidebar_add_folder_button.connect("clicked", self._on_add_folder_button_clicked)
        self.sidebar_inline_add_folder_button.connect("clicked", self._on_add_folder_button_clicked)
        self.sidebar_search_entry.connect("search-changed", self._on_sidebar_search_changed)
        self.sidebar_search_entry.connect("stop-search", self._on_sidebar_search_stop)
        self.sidebar_search_revealer.set_reveal_child(False)
        self.editor_search_entry.connect("search-changed", self._on_editor_search_changed)
        self.editor_search_entry.connect("activate", self._find_next)
        self.editor_replace_entry.connect("activate", self._replace_current_match)
        self._add_entry_key_bindings()
        self.info_type_row.connect("activated", self._on_info_type_activated)
        type_click = Gtk.GestureClick()
        type_click.connect("released", self._on_info_type_clicked)
        self.info_type_row.add_controller(type_click)
        self.info_delete_button.connect("clicked", self._delete_current_note)
        self.info_revealer.connect("notify::child-revealed", self._on_info_child_revealed)
        buffer = self.editor_view.get_buffer()
        buffer.connect("changed", self._on_buffer_changed)
        folders_adjustment = self.sidebar_folders_scroller.get_vadjustment()
        folders_adjustment.connect("changed", self._on_folders_scroller_changed)
        folders_adjustment.connect("value-changed", self._on_folders_scroller_changed)

        self.connect("close-request", self._on_close_request)
        self._refresh_notes()
        self.info_revealer.set_visible(False)
        self._show_empty_state()
        self._apply_theme_mode()
        self._apply_editor_typography()
        self._apply_ruler_settings()

    def _setup_editor(self) -> None:
        self.editor_view = GtkSource.View()
        buffer = GtkSource.Buffer()
        buffer.set_highlight_syntax(True)
        buffer.set_highlight_matching_brackets(True)
        self.editor_view.set_buffer(buffer)
        self.editor_view.set_tab_width(4)
        self.editor_view.set_insert_spaces_instead_of_tabs(True)
        self.editor_view.set_wrap_mode(
            Gtk.WrapMode.WORD_CHAR if self.settings.data.wrap_text else Gtk.WrapMode.NONE
        )
        self.editor_view.set_show_line_numbers(self.settings.data.show_line_numbers)
        self.editor_view.set_monospace(self.settings.data.use_monospace)
        self.editor_view.set_pixels_above_lines(0)
        self.editor_view.set_pixels_below_lines(0)
        self.editor_view.set_pixels_inside_wrap(0)
        self.editor_view.set_top_margin(0)
        self.editor_view.set_bottom_margin(0)
        self.editor_view.set_left_margin(0)
        self.editor_view.set_right_margin(0)
        self.editor_view.set_hexpand(True)
        self.editor_view.set_vexpand(True)
        self.editor_view.add_css_class("editor-view")
        self.editor_view.set_highlight_current_line(True)
        self.editor_view.set_smart_home_end(GtkSource.SmartHomeEndType.ALWAYS)
        self.editor_view.set_hexpand_set(True)
        self.editor_view.connect("insert-emoji", self._on_editor_insert_emoji)
        self._setup_editor_context_menu()
        self.editor_page.set_child(self.editor_view)
        self._search_settings = GtkSource.SearchSettings()
        self._search_settings.set_case_sensitive(False)
        self._search_context = GtkSource.SearchContext.new(buffer, self._search_settings)
        self._search_context.set_highlight(False)
        self._search_context.connect("notify::occurrences-count", self._on_occurrences_count_changed)

        self._style_manager = Adw.StyleManager.get_default()
        self._style_manager.connect("notify::dark", self._on_style_variant_changed)
        self._style_scheme_manager = GtkSource.StyleSchemeManager.get_default()
        self._style_scheme_manager.prepend_search_path(str(STYLES_DIR))
        self._language_manager = GtkSource.LanguageManager.get_default()
        self._update_style_scheme()

    def _setup_language_chooser(self) -> None:
        languages = []
        for language_id in self._language_manager.get_language_ids():
            language = self._language_manager.get_language(language_id)
            if language is not None:
                languages.append((language.get_name(), language_id))

        languages.sort(key=lambda item: item[0].casefold())
        self._language_options = [("Automatic", None)] + languages
        self.language_search_entry.connect("search-changed", self._on_language_search_changed)
        self.language_listbox.connect("row-activated", self._on_language_row_activated)
        self.language_popover.connect("notify::visible", self._on_language_popover_visible_changed)
        self._rebuild_language_list()

    def _setup_actions(self) -> None:
        self._add_simple_action("new-note", self._new_note)
        self._add_simple_action("delete-note", self._delete_current_note)
        self._add_simple_action("close-note", self._close_current_note)
        self._add_simple_action("choose-folder", self._choose_folder)
        self._add_simple_action("print-note", self._print_current_note)
        self._add_simple_action("show-preferences", self._show_preferences)
        self._add_simple_action("show-shortcuts", self._show_shortcuts)
        self._add_simple_action("toggle-fullscreen", self._toggle_fullscreen)
        self._add_simple_action("zoom-in", self._zoom_in)
        self._add_simple_action("zoom-out", self._zoom_out)
        self._add_simple_action("zoom-reset", self._zoom_reset)
        self._add_simple_action("theme-system", self._set_theme_system)
        self._add_simple_action("theme-light", self._set_theme_light)
        self._add_simple_action("theme-dark", self._set_theme_dark)
        self._add_simple_action("focus-sidebar-search", self._focus_sidebar_search)
        self._add_simple_action("toggle-editor-search", self._toggle_editor_search)
        self._add_simple_action("toggle-replace", self._toggle_replace)
        self._add_simple_action("close-search", self._close_editor_search)
        self._add_simple_action("toggle-info", self._toggle_info)
        self._add_simple_action("find-next", self._find_next)
        self._add_simple_action("find-previous", self._find_previous)
        self._add_simple_action("focus-replace", self._focus_replace)
        self._add_simple_action("replace-current", self._replace_current_match)
        self._add_simple_action("replace-all", self._replace_all_matches)
        self._add_simple_action("toggle-sidebar", self._toggle_sidebar)
        self._add_stateful_action(
            "toggle-line-numbers",
            self.settings.data.show_line_numbers,
            self._toggle_line_numbers,
        )
        self._add_stateful_action(
            "toggle-wrap",
            self.settings.data.wrap_text,
            self._toggle_wrap,
        )
        self._add_stateful_action(
            "toggle-monospace",
            self.settings.data.use_monospace,
            self._toggle_monospace,
        )

    def _add_entry_key_bindings(self) -> None:
        search_keys = Gtk.EventControllerKey()
        search_keys.connect("key-pressed", self._on_search_entry_key_pressed)
        self.editor_search_entry.add_controller(search_keys)

        replace_keys = Gtk.EventControllerKey()
        replace_keys.connect("key-pressed", self._on_replace_entry_key_pressed)
        self.editor_replace_entry.add_controller(replace_keys)

        window_shortcuts = Gtk.ShortcutController()
        window_shortcuts.set_scope(Gtk.ShortcutScope.LOCAL)
        window_shortcuts.add_shortcut(
            Gtk.Shortcut.new(
                Gtk.KeyvalTrigger.new(Gdk.KEY_Escape, 0),
                Gtk.CallbackAction.new(self._on_window_escape_shortcut),
            )
        )
        self.add_controller(window_shortcuts)

        window_keys = Gtk.EventControllerKey()
        window_keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        window_keys.connect("key-pressed", self._on_window_key_pressed)
        self.add_controller(window_keys)

    def _setup_menu(self) -> None:
        menu = Gio.Menu()
        custom_item = Gio.MenuItem()
        custom_item.set_attribute_value("custom", GLib.Variant.new_string("theme-selector"))
        theme_section = Gio.Menu()
        theme_section.append_item(custom_item)
        menu.append_section(None, theme_section)

        section = Gio.Menu()
        section.append("Print Note", "win.print-note")
        section.append("Preferences", "win.show-preferences")
        section.append("Keyboard Shortcuts", "win.show-shortcuts")
        menu.append_section(None, section)
        menu.append("About Paper Trail", "app.about")

        theme_section = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        theme_section.add_css_class("papertrail-menu-surface")

        theme_selector = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        theme_selector.set_hexpand(True)
        theme_selector.add_css_class("theme-selector")
        theme_selector.add_css_class("theme-selector-row")

        theme_follow = Gtk.CheckButton()
        theme_follow.set_hexpand(True)
        theme_follow.set_halign(Gtk.Align.CENTER)
        theme_follow.set_focus_on_click(False)
        theme_follow.add_css_class("theme-selector-button")
        theme_follow.add_css_class("follow")
        theme_follow.set_tooltip_text("Follow System Style")
        theme_follow.connect("toggled", self._on_menu_theme_button_toggled, "system")

        theme_light = Gtk.CheckButton()
        theme_light.set_hexpand(True)
        theme_light.set_halign(Gtk.Align.CENTER)
        theme_light.set_focus_on_click(False)
        theme_light.set_group(theme_follow)
        theme_light.add_css_class("theme-selector-button")
        theme_light.add_css_class("light")
        theme_light.set_tooltip_text("Light Style")
        theme_light.connect("toggled", self._on_menu_theme_button_toggled, "light")

        theme_dark = Gtk.CheckButton()
        theme_dark.set_hexpand(True)
        theme_dark.set_halign(Gtk.Align.CENTER)
        theme_dark.set_focus_on_click(False)
        theme_dark.set_group(theme_follow)
        theme_dark.add_css_class("theme-selector-button")
        theme_dark.add_css_class("dark")
        theme_dark.set_tooltip_text("Dark Style")
        theme_dark.connect("toggled", self._on_menu_theme_button_toggled, "dark")

        theme_selector.append(theme_follow)
        theme_selector.append(theme_light)
        theme_selector.append(theme_dark)

        theme_section.append(theme_selector)
        theme_section.append(Gtk.Separator(orientation=Gtk.Orientation.HORIZONTAL))

        zoom_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        zoom_row.set_halign(Gtk.Align.FILL)
        zoom_row.add_css_class("menu-zoom-row")

        zoom_out = Gtk.Button(icon_name="zoom-out-symbolic")
        zoom_out.add_css_class("flat")
        zoom_out.add_css_class("circular")
        zoom_out.connect("clicked", lambda *_args: self._zoom_out())

        zoom_button = Gtk.Button()
        zoom_button.add_css_class("flat")
        zoom_button.set_focus_on_click(False)
        zoom_button.connect("clicked", lambda *_args: self._zoom_reset())

        zoom_label = Gtk.Label(label="100%")
        zoom_label.set_hexpand(True)
        zoom_label.set_xalign(0.5)
        zoom_label.add_css_class("title-4")
        zoom_label.add_css_class("dim-label")
        zoom_button.set_child(zoom_label)

        zoom_in = Gtk.Button(icon_name="zoom-in-symbolic")
        zoom_in.add_css_class("flat")
        zoom_in.add_css_class("circular")
        zoom_in.connect("clicked", lambda *_args: self._zoom_in())

        zoom_row.append(zoom_out)
        zoom_row.append(zoom_button)
        zoom_row.append(zoom_in)
        theme_section.append(zoom_row)

        popover = Gtk.PopoverMenu.new_from_model(menu)
        popover.add_css_class("papertrail-menu-popover")
        popover.add_child(theme_section, "theme-selector")
        self.notes_menu_button.set_popover(popover)

        self._menu_theme_follow = theme_follow
        self._menu_theme_light = theme_light
        self._menu_theme_dark = theme_dark
        self._menu_zoom_button = zoom_button
        self._menu_zoom_label = zoom_label
        self._sync_menu_theme_controls()

    def _setup_editor_context_menu(self) -> None:
        menu = Gio.Menu()

        edit_section = Gio.Menu()
        edit_section.append("Cut", "clipboard.cut")
        edit_section.append("Copy", "clipboard.copy")
        edit_section.append("Paste", "clipboard.paste")
        edit_section.append("Delete", "selection.delete")
        menu.append_section(None, edit_section)

        history_section = Gio.Menu()
        history_section.append("Undo", "text.undo")
        history_section.append("Redo", "text.redo")
        menu.append_section(None, history_section)

        selection_section = Gio.Menu()
        selection_section.append("Select All", "selection.select-all")
        selection_section.append("Insert Emoji", "misc.insert-emoji")

        change_case_menu = Gio.Menu()
        change_case_menu.append("Lowercase", "editor.change-case-lower")
        change_case_menu.append("Uppercase", "editor.change-case-upper")
        change_case_menu.append("Toggle Case", "editor.change-case-toggle")
        change_case_menu.append("Title Case", "editor.change-case-title")
        selection_section.append_submenu("Change Case", change_case_menu)
        menu.append_section(None, selection_section)

        popover = Gtk.PopoverMenu.new_from_model(menu)
        popover.add_css_class("papertrail-menu-popover")
        popover.set_parent(self.editor_view)
        popover.set_has_arrow(True)
        popover.set_autohide(True)
        self._editor_context_popover = popover

        action_group = Gio.SimpleActionGroup()
        self.editor_view.insert_action_group("editor", action_group)

        for name, case_type in (
            ("change-case-lower", GtkSource.ChangeCaseType.LOWER),
            ("change-case-upper", GtkSource.ChangeCaseType.UPPER),
            ("change-case-toggle", GtkSource.ChangeCaseType.TOGGLE),
            ("change-case-title", GtkSource.ChangeCaseType.TITLE),
        ):
            action = Gio.SimpleAction.new(name, None)
            action.connect("activate", self._on_editor_change_case_action, case_type)
            action_group.add_action(action)

        self._editor_context_actions = action_group

        secondary_click = Gtk.GestureClick(button=Gdk.BUTTON_SECONDARY)
        secondary_click.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        secondary_click.connect("pressed", self._on_editor_secondary_click_pressed)
        self.editor_view.add_controller(secondary_click)

        editor_keys = Gtk.EventControllerKey()
        editor_keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        editor_keys.connect("key-pressed", self._on_editor_key_pressed)
        self.editor_view.add_controller(editor_keys)

    def _sync_editor_context_menu_actions(self) -> None:
        if self._editor_context_actions is None:
            return

        buffer = self.editor_view.get_buffer()
        has_selection = buffer.get_has_selection()
        for name in (
            "change-case-lower",
            "change-case-upper",
            "change-case-toggle",
            "change-case-title",
        ):
            action = self._editor_context_actions.lookup_action(name)
            if isinstance(action, Gio.SimpleAction):
                action.set_enabled(has_selection)

    def _popup_editor_context_menu(self, x: float, y: float) -> None:
        if self._editor_context_popover is None:
            return

        self._sync_editor_context_menu_actions()
        self._editor_context_anchor.x = int(x)
        self._editor_context_anchor.y = int(y)
        self._editor_context_anchor.width = 1
        self._editor_context_anchor.height = 1
        self._editor_context_popover.set_pointing_to(self._editor_context_anchor)
        self._editor_context_popover.popdown()
        self._editor_context_popover.popup()

    def _popup_editor_context_menu_at_focus(self) -> None:
        x = self.editor_view.get_allocated_width() / 2
        y = min(self.editor_view.get_allocated_height() / 3, 120)
        self._popup_editor_context_menu(x, y)

    def _ensure_editor_emoji_chooser(self) -> Gtk.EmojiChooser:
        if self._editor_emoji_chooser is None:
            chooser = Gtk.EmojiChooser()
            chooser.add_css_class("papertrail-popover")
            chooser.add_css_class("papertrail-emoji-chooser")
            chooser.set_parent(self.editor_view)
            chooser.set_position(Gtk.PositionType.BOTTOM)
            chooser.connect("emoji-picked", self._on_editor_emoji_picked)
            chooser.connect("closed", lambda *_args: self.editor_view.grab_focus())
            self._editor_emoji_chooser = chooser
        return self._editor_emoji_chooser

    def _show_editor_emoji_chooser(self) -> None:
        chooser = self._ensure_editor_emoji_chooser()
        buffer = self.editor_view.get_buffer()
        insert_iter = buffer.get_iter_at_mark(buffer.get_insert())
        rect = self.editor_view.get_iter_location(insert_iter)
        x, y = self.editor_view.buffer_to_window_coords(
            Gtk.TextWindowType.WIDGET,
            rect.x,
            rect.y + rect.height,
        )
        self._editor_emoji_anchor.x = max(0, x)
        self._editor_emoji_anchor.y = max(0, y)
        self._editor_emoji_anchor.width = max(1, rect.width)
        self._editor_emoji_anchor.height = max(1, rect.height)
        chooser.set_pointing_to(self._editor_emoji_anchor)
        chooser.popdown()
        chooser.popup()

    def _on_editor_change_case_action(
        self,
        _action: Gio.SimpleAction,
        _parameter: GLib.Variant | None,
        case_type: GtkSource.ChangeCaseType,
    ) -> None:
        self.editor_view.emit("change-case", case_type)

    def _on_editor_insert_emoji(self, _view: GtkSource.View) -> None:
        self.editor_view.stop_emission_by_name("insert-emoji")
        self._show_editor_emoji_chooser()

    def _on_editor_emoji_picked(self, chooser: Gtk.EmojiChooser, emoji: str) -> None:
        chooser.popdown()
        self.editor_view.emit("insert-at-cursor", emoji)
        self.editor_view.grab_focus()

    def _on_editor_secondary_click_pressed(
        self,
        gesture: Gtk.GestureClick,
        _n_press: int,
        x: float,
        y: float,
    ) -> None:
        gesture.set_state(Gtk.EventSequenceState.CLAIMED)
        self.editor_view.grab_focus()
        self._popup_editor_context_menu(x, y)

    def _on_editor_key_pressed(
        self,
        _controller: Gtk.EventControllerKey,
        keyval: int,
        _keycode: int,
        state: Gdk.ModifierType,
    ) -> bool:
        if keyval == Gdk.KEY_Menu:
            self._popup_editor_context_menu_at_focus()
            return True

        if keyval == Gdk.KEY_F10 and state & Gdk.ModifierType.SHIFT_MASK:
            self._popup_editor_context_menu_at_focus()
            return True

        return False

    def _install_css(self) -> None:
        provider = Gtk.CssProvider()
        provider.load_from_path(str(UI_DIR / "style.css"))
        Gtk.StyleContext.add_provider_for_display(
            self.get_display(),
            provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION,
        )
        Gtk.StyleContext.add_provider_for_display(
            self.get_display(),
            self._folder_color_provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        )
        Gtk.StyleContext.add_provider_for_display(
            self.get_display(),
            self._scheme_recolor_provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        )

    def _refresh_folder_color_css(self, extra_colors: set[str] | None = None) -> None:
        colors = custom_folder_colors(list((self.settings.data.folder_colors or {}).values()))
        if extra_colors:
            colors.update(custom_folder_colors(list(extra_colors)))
        css = "\n".join(
            f".{folder_color_css_class(color)} {{ color: {color}; }}"
            for color in sorted(colors)
        ).encode("utf-8")
        self._folder_color_provider.load_from_data(css)

    def _add_simple_action(self, name: str, callback) -> None:
        action = Gio.SimpleAction.new(name, None)
        action.connect("activate", callback)
        self.add_action(action)

    def _add_stateful_action(self, name: str, initial: bool, callback) -> None:
        action = Gio.SimpleAction.new_stateful(name, None, GLib.Variant.new_boolean(initial))
        action.connect("change-state", callback)
        self.add_action(action)

    def _set_action_enabled(self, name: str, enabled: bool) -> None:
        action = self.lookup_action(name)
        if isinstance(action, Gio.SimpleAction):
            action.set_enabled(enabled)

    def _apply_theme_mode(self) -> None:
        mode = self.settings.data.theme_mode
        if mode == "light":
            scheme = Adw.ColorScheme.FORCE_LIGHT
        elif mode == "dark":
            scheme = Adw.ColorScheme.FORCE_DARK
        else:
            scheme = Adw.ColorScheme.DEFAULT
        self._style_manager.set_color_scheme(scheme)
        self._sync_menu_theme_controls()
        self._sync_preferences_window()

    def _apply_editor_typography(self) -> None:
        rules: list[str] = []
        percent = int(round(self.settings.data.font_scale * 100))

        if self.settings.data.use_custom_font:
            font_css = self._build_custom_font_css(self.settings.data.custom_font)
            if font_css:
                rules.append(font_css)
        elif percent != 100:
            rules.append(f"font-size: {percent}%;")

        if rules:
            css = f".editor-view, .editor-view text, .recent-note-preview {{ {' '.join(rules)} }}".encode("utf-8")
        else:
            css = b""
        self._editor_zoom_provider.load_from_data(css)
        Gtk.StyleContext.add_provider_for_display(
            self.get_display(),
            self._editor_zoom_provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        )
        self._sync_preferences_window()

    def _build_custom_font_css(self, font_spec: str) -> str:
        description = Pango.FontDescription.from_string(font_spec or "")
        declarations: list[str] = []

        family = description.get_family()
        if family:
            css_family = family.replace("\\", "\\\\").replace("'", "\\'")
            declarations.append(f"font-family: '{css_family}';")

        style_map = {
            Pango.Style.NORMAL: "normal",
            Pango.Style.OBLIQUE: "oblique",
            Pango.Style.ITALIC: "italic",
        }
        style = style_map.get(description.get_style(), "normal")
        declarations.append(f"font-style: {style};")
        declarations.append(f"font-weight: {int(description.get_weight())};")

        size = description.get_size()
        if size > 0:
            scaled_size = (size / Pango.SCALE) * self.settings.data.font_scale
            unit = "px" if description.get_size_is_absolute() else "pt"
            declarations.append(f"font-size: {scaled_size:.2f}{unit};")
        elif self.settings.data.font_scale != 1.0:
            declarations.append(f"font-size: {int(round(self.settings.data.font_scale * 100))}%;")

        return " ".join(declarations)

    def _apply_ruler_settings(self) -> None:
        self.editor_view.set_show_right_margin(self.settings.data.show_ruler)
        self.editor_view.set_right_margin_position(100)
        self._sync_preferences_window()

    def _refresh_folders(self) -> None:
        self._refresh_folder_color_css()
        child = self.folder_list.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            self.folder_list.remove(child)
            child = next_child

        all_row = FolderRow(
            ALL_FOLDERS_PATH,
            "preset-11",
            title="All",
            badge_text="All",
            tooltip_text="Show notes from all folders",
            menu_enabled=False,
            drag_enabled=False,
        )
        all_row.connect("clicked", self._on_all_folders_row_clicked)
        self.folder_list.append(all_row)
        if self._show_all_folders:
            self._select_folder_row(all_row)

        active_folder = self.repository.notes_dir
        for folder in self.settings.note_folders:
            row = FolderRow(folder, self.settings.get_folder_color(folder))
            row.update(folder)
            row.connect("clicked", self._on_folder_row_clicked, folder)
            row.connect("edit-requested", self._on_folder_row_edit_requested, folder)
            row.connect("close-requested", self._on_folder_row_close_requested, folder)
            row.connect("reorder-requested", self._on_folder_row_reorder_requested, folder)
            self.folder_list.append(row)
            if not self._show_all_folders and folder == active_folder:
                self._select_folder_row(row)

        self._queue_sync_folder_add_button_visibility()

    def _queue_sync_folder_add_button_visibility(self) -> None:
        if self._folder_add_visibility_source_id is not None:
            return
        self._folder_add_visibility_source_id = GLib.idle_add(self._sync_folder_add_button_visibility)

    def _sync_folder_add_button_visibility(self) -> bool:
        self._folder_add_visibility_source_id = None

        folder_height = self.folder_list.get_allocated_height()
        viewport_height = self.sidebar_folders_viewport.get_allocated_height()
        inline_height = self.sidebar_inline_add_box.get_allocated_height() or self.sidebar_folder_add_box.get_allocated_height()

        show_inline = folder_height > 0 and viewport_height > 0 and folder_height + inline_height <= viewport_height
        self.sidebar_inline_add_box.set_visible(show_inline)
        self.sidebar_folder_add_box.set_visible(not show_inline)
        return GLib.SOURCE_REMOVE

    def _on_folders_scroller_changed(self, *_args) -> None:
        self._queue_sync_folder_add_button_visibility()

    def _select_folder_row(self, row: FolderRow | None) -> None:
        if self._active_folder_row is row:
            return
        if self._active_folder_row is not None:
            self._active_folder_row.set_active(False)
        self._active_folder_row = row
        if row is not None:
            row.set_active(True)

    def _sync_menu_theme_controls(self) -> None:
        mode = self.settings.data.theme_mode
        if self._menu_theme_follow is not None:
            self._menu_theme_follow.set_active(mode == "system")
        if self._menu_theme_light is not None:
            self._menu_theme_light.set_active(mode == "light")
        if self._menu_theme_dark is not None:
            self._menu_theme_dark.set_active(mode == "dark")
        percent = int(round(self.settings.data.font_scale * 100))
        if self._menu_zoom_button is not None:
            self._menu_zoom_button.set_sensitive(percent != 100)
        if self._menu_zoom_label is not None:
            self._menu_zoom_label.set_text(f"{percent}%")
            if percent == 100:
                self._menu_zoom_label.add_css_class("dim-label")
            else:
                self._menu_zoom_label.remove_css_class("dim-label")

    def _refresh_notes(self, selected_path: Path | None = None) -> None:
        self._refresh_folders()
        self._notes = sorted(
            self.repository.list_notes(),
            key=lambda note: (
                not self.settings.is_note_pinned(note.path),
                -note.modified_at.timestamp(),
            ),
        )
        self._rebuild_recent_notes_grid()
        query = self.sidebar_search_entry.get_text().strip().casefold()
        visible_notes = self._get_visible_notes(query)

        row_by_path: dict[Path, NoteRow] = {}
        child = self.note_list.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            if isinstance(child, NoteRow):
                row_by_path[child.note_path] = child
            self.note_list.remove(child)
            child = next_child

        for note in visible_notes:
            row = row_by_path.get(note.path)
            if row is None:
                row = NoteRow(note)
                row.connect("activated", self._on_row_activated)
                row.connect("rename-submitted", self._on_sidebar_row_rename_submitted)
                row.connect("pin-toggled", self._on_sidebar_row_pin_toggled)
                row.connect("open-folder-requested", self._on_sidebar_row_open_folder_requested)
                row.connect("move-to-folder-requested", self._on_sidebar_row_move_to_folder_requested)
                row.connect("delete-requested", self._on_sidebar_row_delete_requested)
            row.update(note)
            row.set_language_label(self._get_note_language_label(note))
            row.set_pinned(self.settings.is_note_pinned(note.path))
            row.set_folder_color_token(self.settings.get_folder_color(note.path.parent))
            row.set_move_targets(
                [
                    (str(folder), self.settings.get_folder_color(folder))
                    for folder in self.settings.note_folders
                    if folder != note.path.parent
                ]
            )
            self.note_list.append(row)

        folder_name = "All" if self._show_all_folders else (self.repository.notes_dir.name or str(self.repository.notes_dir))
        note_count = f"{len(self._notes)} note" if len(self._notes) == 1 else f"{len(self._notes)} notes"
        if query:
            match_count = f"{len(visible_notes)} match" if len(visible_notes) == 1 else f"{len(visible_notes)} matches"
            self.sidebar_subtitle_label.set_text(f"{folder_name} - {match_count}")
        else:
            count_label = f"{len(visible_notes)} note" if len(visible_notes) == 1 else f"{len(visible_notes)} notes"
            self.sidebar_subtitle_label.set_text(f"{folder_name} - {count_label if self._show_all_folders else note_count}")

        if self.current_note and not self.current_note.path.exists():
            self.current_note = None
            self._show_empty_state()

        if selected_path is None and self.current_note:
            selected_path = self.current_note.path

        if selected_path:
            row = self._find_row_for_path(selected_path)
            if row:
                self._select_row(row)
                return

        if selected_path is None and self.current_note is None:
            self._select_row(None)
            return

        if selected_path is not None:
            self._select_row(None)

    def _matches_note_query(self, note: NoteRecord, query: str) -> bool:
        return (
            query in note.title.casefold()
            or query in note.body.casefold()
            or query in note.path.name.casefold()
        )

    def _get_visible_notes(self, query: str) -> list[NoteRecord]:
        if query:
            return self._search_notes_across_folders(query)
        if self._show_all_folders:
            return self._search_notes_across_folders("")
        return self._notes

    def _search_notes_across_folders(self, query: str) -> list[NoteRecord]:
        matches: list[NoteRecord] = []
        for folder in self.settings.note_folders:
            if folder == self.repository.notes_dir:
                notes = self._notes
            else:
                if not folder.exists():
                    continue
                try:
                    notes = [
                        self.repository.load_note(path)
                        for path in sorted(folder.iterdir())
                        if path.is_file()
                    ]
                except OSError:
                    continue

            for note in notes:
                if not query or self._matches_note_query(note, query):
                    matches.append(note)

        matches.sort(
            key=lambda note: (
                not self.settings.is_note_pinned(note.path),
                -note.modified_at.timestamp(),
            )
        )
        return matches

    def _get_note_for_path(self, path: Path) -> NoteRecord | None:
        if self.current_note is not None and self.current_note.path == path:
            return self.current_note
        for note in self._notes:
            if note.path == path:
                return note
        if path.exists():
            return self.repository.load_note(path)
        return None

    def _select_row(self, row: NoteRow | None) -> None:
        if self._active_note_row is row:
            return
        if self._active_note_row is not None:
            self._active_note_row.set_active(False)
        self._active_note_row = row
        if row is not None:
            row.set_active(True)

    def _set_active_folder(self, folder: Path) -> None:
        if not self._show_all_folders and folder == self.repository.notes_dir:
            return
        self._flush_pending_save()
        self._show_all_folders = False
        self.settings.set_notes_dir(folder)
        self.repository.set_notes_dir(folder)
        self.current_note = None
        self._refresh_notes()
        self._show_empty_state()
        self._sync_preferences_window()

    def _set_show_all_folders(self) -> None:
        if self._show_all_folders:
            return
        self._flush_pending_save()
        self._show_all_folders = True
        self.current_note = None
        self._refresh_notes()
        self._show_empty_state()

    def _on_all_folders_row_clicked(self, _button: FolderRow) -> None:
        self._set_show_all_folders()

    def _on_folder_row_clicked(self, _button: FolderRow, folder: Path) -> None:
        self._set_active_folder(folder)

    def _on_folder_row_reorder_requested(
        self,
        _row: FolderRow,
        dragged_path: str,
        before: bool,
        target_folder: Path,
    ) -> None:
        dragged_folder = Path(dragged_path).expanduser()
        folders = list(self.settings.note_folders)
        if dragged_folder == target_folder or dragged_folder not in folders or target_folder not in folders:
            return

        folders.remove(dragged_folder)
        target_index = folders.index(target_folder)
        insert_at = target_index if before else target_index + 1
        folders.insert(insert_at, dragged_folder)
        self.settings.reorder_notes_dirs(folders)
        self._refresh_folders()
        self._sync_preferences_window()

    def _on_add_folder_button_clicked(self, *_args) -> None:
        self._choose_folder()

    def _on_sidebar_row_pin_toggled(self, row: NoteRow, pinned: bool) -> None:
        self.settings.set_note_pinned(row.note_path, pinned)
        self._refresh_notes(selected_path=row.note_path)

    def _close_current_note(self, *_args) -> None:
        if self.current_note is None:
            return
        self._flush_pending_save()
        self.current_note = None
        self._select_row(None)
        self._show_empty_state()

    def _on_folder_row_close_requested(self, _row: FolderRow, folder: Path) -> None:
        self._close_folder(folder)

    def _close_folder(self, folder: Path) -> None:
        if len(self.settings.note_folders) <= 1:
            return
        self._flush_pending_save()
        self.settings.remove_notes_dir(folder)
        self.repository.set_notes_dir(self.settings.notes_dir)
        self.current_note = None
        self._refresh_notes()
        self._show_empty_state()
        self._sync_preferences_window()

    def _on_folder_row_edit_requested(self, _row: FolderRow, folder: Path) -> None:
        self._present_folder_edit_window(folder)

    def _find_row_for_path(self, path: Path) -> NoteRow | None:
        child = self.note_list.get_first_child()
        while child is not None:
            if isinstance(child, NoteRow) and child.note_path == path:
                return child
            child = child.get_next_sibling()
        return None

    def _show_note(self, note: NoteRecord) -> None:
        self._flush_pending_save()
        self.current_note = note
        self._loading_note = True
        buffer = self.editor_view.get_buffer()
        self._apply_note_language(note)
        buffer.set_text(note.body)
        self._loading_note = False
        self.editor_stack.set_visible_child(self.editor_page)
        self._set_title_label(note.title)
        self._set_filename_label(note.path.name)
        self._sync_language_selection(note)
        self.info_button.set_sensitive(True)
        self.language_popout_button.set_sensitive(True)
        self.info_delete_button.set_sensitive(True)
        for action_name in (
            "close-note",
            "delete-note",
            "print-note",
            "toggle-info",
            "toggle-editor-search",
            "focus-replace",
            "find-next",
            "find-previous",
            "replace-current",
            "replace-all",
        ):
            self._set_action_enabled(action_name, True)
        self._update_info_panel(note)
        self._refresh_search_status()
        self._update_sidebar_selection(note.path)
        self.editor_view.grab_focus()

    def _show_empty_state(self) -> None:
        self.editor_stack.set_visible_child(self.empty_page)
        has_notes = bool(self._notes)
        self.empty_state_stack.set_visible_child(self.recent_notes_page if has_notes else self.empty_folder_page)
        self._set_title_label("Recent Notes" if has_notes else "Get started")
        self._set_filename_label("")
        self.editor_view.get_buffer().set_language(None)
        self._sync_language_selection(None)
        self.info_button.set_sensitive(False)
        self.language_popout_button.set_sensitive(False)
        self.info_delete_button.set_sensitive(False)
        for action_name in (
            "close-note",
            "delete-note",
            "print-note",
            "toggle-info",
            "toggle-editor-search",
            "focus-replace",
            "find-next",
            "find-previous",
            "replace-current",
            "replace-all",
        ):
            self._set_action_enabled(action_name, False)
        self.info_button.set_active(False)
        self.info_revealer.set_reveal_child(False)
        self._clear_info_panel()
        self._close_editor_search()

    def _rebuild_recent_notes_grid(self) -> None:
        child = self.recent_notes_grid.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            self.recent_notes_grid.remove(child)
            child = next_child

        recent_notes = self._recent_notes_source()[:9]
        self.recent_notes_grid.set_visible(bool(recent_notes))
        self.recent_notes_subtitle_label.set_text("Open one of your latest notes or create a new one.")

        for note in recent_notes:
            tile = Gtk.Button()
            tile.add_css_class("flat")
            tile.add_css_class("recent-note-tile")
            tile.set_halign(Gtk.Align.FILL)
            tile.set_valign(Gtk.Align.FILL)
            tile.set_hexpand(True)
            tile.set_vexpand(True)
            tile.set_size_request(-1, 210)
            tile.connect("clicked", self._on_recent_note_clicked, note.path)

            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
            box.set_vexpand(True)
            box.set_margin_top(16)
            box.set_margin_bottom(16)
            box.set_margin_start(16)
            box.set_margin_end(16)

            title = Gtk.Label(xalign=0)
            title.set_text(note.title)
            title.set_ellipsize(Pango.EllipsizeMode.END)
            title.add_css_class("title-4")
            title.add_css_class("recent-note-title")

            filename = Gtk.Label(xalign=0)
            filename.set_text(note.path.name)
            filename.set_ellipsize(Pango.EllipsizeMode.END)
            filename.add_css_class("caption")
            filename.add_css_class("dim-label")
            filename.add_css_class("recent-note-filename")

            preview = Gtk.TextView()
            preview_buffer = preview.get_buffer()
            preview_buffer.set_text(note.preview)
            preview.set_editable(False)
            preview.set_cursor_visible(False)
            preview.set_can_focus(False)
            preview.set_wrap_mode(Gtk.WrapMode.NONE)
            preview.set_left_margin(0)
            preview.set_right_margin(0)
            preview.set_top_margin(0)
            preview.set_bottom_margin(0)
            preview.set_pixels_above_lines(0)
            preview.set_pixels_below_lines(0)
            preview.set_vexpand(True)
            preview.set_hexpand(True)
            preview.set_valign(Gtk.Align.FILL)
            preview.add_css_class("recent-note-preview")

            modified = Gtk.Label(xalign=0)
            modified.set_text(note.modified_at.strftime("%b %d, %H:%M"))
            modified.add_css_class("caption")
            modified.add_css_class("dim-label")

            spacer = Gtk.Box()
            spacer.set_vexpand(True)

            box.append(title)
            box.append(filename)
            box.append(preview)
            box.append(spacer)
            box.append(modified)
            tile.set_child(box)
            self.recent_notes_grid.insert(tile, -1)

    def _recent_notes_source(self) -> list[NoteRecord]:
        if self._show_all_folders:
            return self._search_notes_across_folders("")
        return self._notes

    def _on_recent_note_clicked(self, _button: Gtk.Button, path: Path) -> None:
        note = next((item for item in self._notes if item.path == path), None)
        if note is not None:
            self._show_note(note)

    def _update_sidebar_selection(self, path: Path) -> None:
        row = self._find_row_for_path(path)
        if row:
            self._select_row(row)

    def _update_sidebar_row(self, note: NoteRecord) -> None:
        row = self._find_row_for_path(note.path)
        if row is not None:
            row.update(note)
            row.set_language_label(self._get_note_language_label(note))
            row.set_pinned(self.settings.is_note_pinned(note.path))
            row.set_folder_color_token(self.settings.get_folder_color(note.path.parent))

    def _replace_cached_note(self, note: NoteRecord, old_path: Path | None = None) -> None:
        target = old_path or note.path
        for index, existing in enumerate(self._notes):
            if existing.path == target:
                self._notes[index] = note
                return
        self._notes.append(note)

    def _new_note(self, *_args) -> None:
        note = self.repository.create_note()
        self.current_note = None
        self._refresh_notes(selected_path=note.path)
        self._show_note(note)

    def _delete_current_note(self, *_args) -> None:
        if self.current_note is None:
            return

        self._confirm_delete_note(self.current_note.path)

    def _confirm_delete_note(self, path: Path) -> None:
        if not path.exists():
            return

        dialog = Adw.MessageDialog.new(
            self,
            "Delete this note?",
            "The note will be removed from the local notes folder.",
        )
        dialog.add_css_class("papertrail-dialog")
        dialog.add_response("cancel", "Cancel")
        dialog.add_response("delete", "Delete")
        dialog.set_response_appearance("delete", Adw.ResponseAppearance.DESTRUCTIVE)
        dialog.set_default_response("cancel")
        dialog.connect("response", self._on_delete_response, path)
        dialog.present()

    def _open_note_folder(self, note_path: Path) -> None:
        launcher = Gtk.FileLauncher.new(Gio.File.new_for_path(str(note_path)))
        launcher.open_containing_folder(self, None, self._on_open_note_folder_complete)

    def _on_open_note_folder_complete(
        self,
        launcher: Gtk.FileLauncher,
        result: Gio.AsyncResult,
    ) -> None:
        try:
            launcher.open_containing_folder_finish(result)
        except GLib.Error as error:
            self._show_error_dialog(
                "Could Not Open Folder",
                str(error) or "The note folder could not be opened.",
            )

    def _on_delete_response(
        self,
        _dialog: Adw.MessageDialog,
        response: str,
        path: Path,
    ) -> None:
        if response != "delete":
            return

        deleted_current = self.current_note is not None and self.current_note.path == path

        self.repository.delete_note(path)
        self.settings.delete_language_override(path)
        self.settings.delete_pinned_note(path)
        if deleted_current:
            self.current_note = None
        self._refresh_notes(selected_path=None if deleted_current else self.current_note.path if self.current_note else None)
        if not deleted_current:
            return

        first_row = self.note_list.get_first_child()
        if isinstance(first_row, NoteRow):
            note = next(note for note in self._notes if note.path == first_row.note_path)
            self._show_note(note)
        else:
            self._show_empty_state()

    def _show_error_dialog(self, heading: str, body: str) -> None:
        dialog = Adw.MessageDialog.new(self, heading, body)
        dialog.add_css_class("papertrail-dialog")
        dialog.add_response("ok", "OK")
        dialog.set_default_response("ok")
        dialog.set_close_response("ok")
        dialog.present()

    def _choose_folder(self, *_args) -> None:
        chooser = Gtk.FileChooserNative.new(
            "Choose Notes Folder",
            self,
            Gtk.FileChooserAction.SELECT_FOLDER,
            "Open",
            "Cancel",
        )
        current = Gio.File.new_for_path(str(self.repository.notes_dir))
        chooser.set_current_folder(current)
        chooser.connect("response", self._on_choose_folder_response)
        chooser.show()

    def _on_choose_folder_response(self, chooser: Gtk.FileChooserNative, response: int) -> None:
        if response != Gtk.ResponseType.ACCEPT:
            chooser.destroy()
            return

        file = chooser.get_file()
        chooser.destroy()
        if file is None:
            return

        path = Path(file.get_path())
        self.settings.add_notes_dir(path, activate=True)
        self._set_active_folder(path)

    def _focus_sidebar_search(self, *_args) -> None:
        self.sidebar_search_revealer.set_reveal_child(True)
        self.sidebar_search_entry.grab_focus()
        self.sidebar_search_entry.select_region(0, -1)

    def _toggle_editor_search(self, *_args) -> None:
        if self.current_note is None:
            return
        self.editor_search_revealer.set_reveal_child(True)
        self.editor_replace_revealer.set_reveal_child(False)
        self.editor_replace_toggle_button.set_active(False)
        self._seed_search_entry_from_selection()
        self.editor_search_entry.grab_focus()
        self.editor_search_entry.select_region(0, -1)

    def _focus_replace(self, *_args) -> None:
        if self.current_note is None:
            return
        self.editor_search_revealer.set_reveal_child(True)
        self.editor_replace_revealer.set_reveal_child(True)
        self.editor_replace_toggle_button.set_active(True)
        self._seed_search_entry_from_selection()
        self.editor_replace_entry.grab_focus()
        self.editor_replace_entry.select_region(0, -1)

    def _seed_search_entry_from_selection(self) -> None:
        buffer = self.editor_view.get_buffer()
        if not buffer.get_has_selection():
            return
        start_iter, end_iter = buffer.get_selection_bounds()
        selected = buffer.get_text(start_iter, end_iter, True)
        if selected and "\n" not in selected:
            self.editor_search_entry.set_text(selected)

    def _close_editor_search(self, *_args) -> None:
        self.editor_search_revealer.set_reveal_child(False)
        self.editor_replace_revealer.set_reveal_child(False)
        self.editor_replace_toggle_button.set_active(False)
        self.editor_search_entry.set_text("")
        self.editor_replace_entry.set_text("")
        self.editor_search_status_label.set_text("")
        if self._search_settings is not None:
            self._search_settings.set_search_text("")
        if self._search_context is not None:
            self._search_context.set_highlight(False)

    def _toggle_replace(self, *_args) -> None:
        if self.current_note is None:
            return
        reveal = not self.editor_replace_revealer.get_reveal_child()
        self.editor_replace_revealer.set_reveal_child(reveal)
        self.editor_replace_toggle_button.set_active(reveal)
        if reveal:
            self.editor_replace_entry.grab_focus()
            self.editor_replace_entry.select_region(0, -1)
        else:
            self.editor_search_entry.grab_focus()

    def _toggle_sidebar(self, *_args) -> None:
        self._sidebar_visible = not self._sidebar_visible
        self.sidebar_panel.set_visible(self._sidebar_visible)
        self.sidebar_divider.set_visible(self._sidebar_visible)

    def _toggle_line_numbers(self, action: Gio.SimpleAction, value: GLib.Variant) -> None:
        enabled = value.get_boolean()
        action.set_state(value)
        self.editor_view.set_show_line_numbers(enabled)
        self.settings.data.show_line_numbers = enabled
        self.settings.save()
        self._sync_preferences_window()

    def _toggle_wrap(self, action: Gio.SimpleAction, value: GLib.Variant) -> None:
        enabled = value.get_boolean()
        action.set_state(value)
        self.editor_view.set_wrap_mode(Gtk.WrapMode.WORD_CHAR if enabled else Gtk.WrapMode.NONE)
        self.settings.data.wrap_text = enabled
        self.settings.save()
        self._sync_preferences_window()

    def _toggle_monospace(self, action: Gio.SimpleAction, value: GLib.Variant) -> None:
        enabled = value.get_boolean()
        action.set_state(value)
        self.editor_view.set_monospace(enabled)
        self.settings.data.use_monospace = enabled
        self.settings.save()
        self._sync_preferences_window()

    def _set_theme_system(self, *_args) -> None:
        self.settings.data.theme_mode = "system"
        self.settings.save()
        self._apply_theme_mode()

    def _set_theme_light(self, *_args) -> None:
        self.settings.data.theme_mode = "light"
        self.settings.save()
        self._apply_theme_mode()

    def _set_theme_dark(self, *_args) -> None:
        self.settings.data.theme_mode = "dark"
        self.settings.save()
        self._apply_theme_mode()

    def _zoom_in(self, *_args) -> None:
        percent = min(250, int(round(self.settings.data.font_scale * 100)) + 10)
        self.settings.data.font_scale = percent / 100
        self.settings.save()
        self._apply_editor_typography()
        self._sync_menu_theme_controls()

    def _zoom_out(self, *_args) -> None:
        percent = max(70, int(round(self.settings.data.font_scale * 100)) - 10)
        self.settings.data.font_scale = percent / 100
        self.settings.save()
        self._apply_editor_typography()
        self._sync_menu_theme_controls()

    def _zoom_reset(self, *_args) -> None:
        self.settings.data.font_scale = 1.0
        self.settings.save()
        self._apply_editor_typography()
        self._sync_menu_theme_controls()

    def _on_menu_theme_button_toggled(
        self,
        button: Gtk.CheckButton,
        mode: str,
        *_args,
    ) -> None:
        if not button.get_active():
            return
        if self.settings.data.theme_mode == mode:
            return
        if mode == "light":
            self._set_theme_light()
        elif mode == "dark":
            self._set_theme_dark()
        else:
            self._set_theme_system()

    def _toggle_fullscreen(self, *_args) -> None:
        if self.is_fullscreen():
            self.unfullscreen()
        else:
            self.fullscreen()

    def _print_current_note(self, *_args) -> None:
        if self.current_note is None:
            return

        compositor = GtkSource.PrintCompositor.new_from_view(self.editor_view)
        compositor.set_wrap_mode(self.editor_view.get_wrap_mode())
        operation = Gtk.PrintOperation()

        def on_begin_print(_operation: Gtk.PrintOperation, context: Gtk.PrintContext) -> None:
            compositor.paginate(context)
            operation.set_n_pages(compositor.get_n_pages())

        def on_draw_page(
            _operation: Gtk.PrintOperation,
            context: Gtk.PrintContext,
            page_nr: int,
        ) -> None:
            compositor.draw_page(context, page_nr)

        operation.connect("begin-print", on_begin_print)
        operation.connect("draw-page", on_draw_page)
        operation.run(Gtk.PrintOperationAction.PRINT_DIALOG, self)

    def _show_preferences(self, *_args) -> None:
        if self._preferences_window is None:
            self._preferences_window = self._build_preferences_window()
        self._sync_preferences_dialog_size()
        self._ensure_preferences_size_sync()
        self._sync_preferences_window()
        self._preferences_window.present(self)

    def _sync_preferences_dialog_size(self, window_height: int | None = None) -> None:
        if self._preferences_window is None:
            return
        window_height = window_height or self.get_height() or self.settings.data.height
        self._preferences_window.set_content_height(max(320, int(window_height * 0.8)))

    def _ensure_preferences_size_sync(self) -> None:
        if self._preferences_size_sync_source_id is not None:
            return
        self._preferences_size_sync_source_id = GLib.timeout_add(100, self._sync_preferences_dialog_size_poll)

    def _sync_preferences_dialog_size_poll(self) -> bool:
        if self._preferences_window is None or not self._preferences_window.get_visible():
            self._preferences_size_sync_source_id = None
            return False
        self._sync_preferences_dialog_size()
        return True

    def _show_shortcuts(self, *_args) -> None:
        if self._shortcuts_window is None:
            self._shortcuts_window = self._build_shortcuts_window()
        self._sync_shortcuts_window("")
        self._shortcuts_window.search_entry.set_text("")
        self._shortcuts_window.present(self)

    def _build_shortcuts_window(self) -> Adw.Dialog:
        window = Adw.Dialog()
        window.add_css_class("papertrail-dialog")
        window.set_title("Keyboard Shortcuts")
        window.set_content_width(560)
        window.set_content_height(640)
        window.set_follows_content_size(False)
        window.set_presentation_mode(Adw.DialogPresentationMode.FLOATING)

        shortcuts = Gtk.ShortcutController()
        shortcuts.set_scope(Gtk.ShortcutScope.LOCAL)
        shortcuts.add_shortcut(
            Gtk.Shortcut.new(
                Gtk.KeyvalTrigger.new(Gdk.KEY_Escape, 0),
                Gtk.CallbackAction.new(lambda *_args: window.close() or True),
            )
        )
        window.add_controller(shortcuts)

        window_keys = Gtk.EventControllerKey()
        window_keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        window_keys.connect(
            "key-pressed",
            lambda _controller, keyval, _keycode, _state: (
                window.close() or True if keyval == Gdk.KEY_Escape else False
            ),
        )
        window.add_controller(window_keys)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        content.set_margin_top(12)
        content.set_margin_bottom(12)
        content.set_margin_start(12)
        content.set_margin_end(12)

        search_entry = Gtk.SearchEntry()
        search_entry.set_placeholder_text("Search shortcuts")

        listbox = Gtk.ListBox()
        listbox.add_css_class("boxed-list")
        listbox.set_selection_mode(Gtk.SelectionMode.NONE)

        scroller = Gtk.ScrolledWindow()
        scroller.set_vexpand(True)
        scroller.set_child(listbox)

        content.append(search_entry)
        content.append(scroller)
        window.set_child(content)

        shortcuts = [
            ("New Note", "Ctrl+N", "Create a new note"),
            ("Close Note", "Ctrl+W", "Unselect the current note"),
            ("Print Note", "Ctrl+P", "Print the current note"),
            ("Preferences", "Ctrl+,", "Open preferences"),
            ("Quit", "Ctrl+Q", "Quit the app"),
            ("Sidebar Search", "Ctrl+K", "Focus note search"),
            ("Toggle Sidebar", "F9 / Ctrl+B", "Show or hide the notes sidebar"),
            ("Toggle Info Panel", "F10 / Ctrl+I", "Show or hide the info panel"),
            ("Toggle Fullscreen", "F11", "Toggle fullscreen mode"),
            ("Find", "Ctrl+F", "Open search"),
            ("Replace", "Ctrl+H", "Open replace"),
            ("Next Match", "F3 / Ctrl+G", "Go to next search match"),
            ("Previous Match", "Shift+F3 / Ctrl+Shift+G", "Go to previous search match"),
            ("Replace Current", "Ctrl+Enter", "Replace the current match"),
            ("Replace All", "Ctrl+Shift+Enter", "Replace all matches"),
            ("Close Search", "Escape", "Close the search bar"),
            ("Delete Note", "Ctrl+Delete", "Delete the current note"),
            ("Zoom In", "Ctrl++", "Increase editor text size"),
            ("Zoom Out", "Ctrl+-", "Decrease editor text size"),
            ("Reset Zoom", "Ctrl+0", "Reset editor text size"),
            ("Toggle Line Numbers", "F7", "Show or hide line numbers"),
            ("Toggle Wrap", "Alt+Z", "Toggle line wrapping"),
            ("Toggle Monospace", "Ctrl+M", "Toggle monospace font"),
            ("About", "F1", "Open the About window"),
        ]

        for title, accel, description in shortcuts:
            row = Adw.ActionRow(title=title, subtitle=description)
            row.search_text = f"{title} {accel} {description}".casefold()
            accel_label = Gtk.Label(label=accel, xalign=1)
            accel_label.add_css_class("dim-label")
            accel_label.add_css_class("caption")
            row.add_suffix(accel_label)
            listbox.append(row)

        search_entry.connect(
            "search-changed",
            lambda entry: self._sync_shortcuts_window(entry.get_text()),
        )

        window.search_entry = search_entry
        window.listbox = listbox
        return window

    def _sync_shortcuts_window(self, query: str) -> None:
        if self._shortcuts_window is None:
            return
        term = query.strip().casefold()
        child = self._shortcuts_window.listbox.get_first_child()
        while child is not None:
            child.set_visible(not term or term in getattr(child, "search_text", ""))
            child = child.get_next_sibling()

    def _build_preferences_window(self) -> Adw.PreferencesDialog:
        window = Adw.PreferencesDialog()
        window.add_css_class("papertrail-dialog")
        window.add_css_class("papertrail-preferences")
        window.set_title("Preferences")
        window.set_content_width(620)
        window.set_content_height(420)
        window.set_follows_content_size(False)
        window.set_presentation_mode(Adw.DialogPresentationMode.FLOATING)

        page = Adw.PreferencesPage()
        page.add_css_class("papertrail-preferences-page")

        appearance = Adw.PreferencesGroup(title="Appearance")
        preview = GtkSource.View()
        preview_buffer = GtkSource.Buffer()
        preview_buffer.set_text(SCHEME_PREVIEW_TEXT)
        preview_language = self._language_manager.guess_language("preview.md", None)
        if preview_language is not None:
            preview_buffer.set_language(preview_language)
            preview_buffer.set_highlight_syntax(True)
        preview.set_buffer(preview_buffer)
        preview.set_editable(False)
        preview.set_cursor_visible(False)
        preview.set_can_focus(False)
        preview.set_top_margin(8)
        preview.set_bottom_margin(8)
        preview.set_left_margin(12)
        preview.set_right_margin(12)
        preview.set_right_margin_position(30)
        preview.set_monospace(True)
        preview.set_show_line_numbers(True)
        preview.set_wrap_mode(Gtk.WrapMode.WORD_CHAR)
        preview.set_hexpand(True)
        preview.set_vexpand(False)
        preview.set_margin_top(10)
        preview.set_margin_bottom(20)
        preview.add_css_class("preview")
        preview.add_css_class("card")
        appearance.add(preview)

        scheme_holder = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        scheme_flow = self._build_scheme_preview_grid()
        scheme_holder.append(scheme_flow)
        appearance.add(scheme_holder)

        font_settings = Adw.PreferencesGroup()

        custom_font_picker_row = Adw.ActionRow(title="Editor Font")
        custom_font_picker_row.set_activatable(True)
        custom_font_picker_row.connect("activated", self._on_preferences_pick_custom_font)
        custom_font_picker_row.add_suffix(Gtk.Image.new_from_icon_name("go-next-symbolic"))
        font_dialog = Gtk.FontDialog()
        font_dialog.set_modal(True)
        font_settings.add(custom_font_picker_row)

        editor = Adw.PreferencesGroup(title="Editor")
        line_numbers_row = Adw.SwitchRow(title="Show Line Numbers")
        line_numbers_row.connect("notify::active", self._on_preferences_line_numbers_changed)
        wrap_row = Adw.SwitchRow(title="Wrap Text")
        wrap_row.connect("notify::active", self._on_preferences_wrap_changed)
        ruler_row = Adw.SwitchRow(title="Show Ruler")
        ruler_row.set_subtitle("Display a vertical guide line in the editor.")
        ruler_row.connect("notify::active", self._on_preferences_ruler_changed)
        editor.add(line_numbers_row)
        editor.add(wrap_row)
        editor.add(ruler_row)

        workspace = Adw.PreferencesGroup(title="Folders")
        add_folder_row = Adw.ActionRow(title="Add Folder…")
        add_folder_row.set_activatable(True)
        add_folder_row.add_prefix(Gtk.Image.new_from_icon_name("list-add-symbolic"))
        add_folder_row.connect("activated", lambda *_args: self._choose_folder())
        folders_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        folders_box.add_css_class("preferences-folder-list")
        workspace.add(add_folder_row)
        workspace.add(folders_box)

        page.add(appearance)
        page.add(font_settings)
        page.add(editor)
        page.add(workspace)
        window.add(page)

        window.appearance_preview = preview
        window.scheme_holder = scheme_holder
        window.scheme_flow = scheme_flow
        window.custom_font_picker_row = custom_font_picker_row
        window.font_dialog = font_dialog
        window.line_numbers_row = line_numbers_row
        window.wrap_row = wrap_row
        window.ruler_row = ruler_row
        window.folders_box = folders_box
        return window

    def _sync_preferences_window(self) -> None:
        if self._preferences_window is None:
            return
        self._syncing_preferences = True
        self._sync_scheme_previews()
        font_label = self.settings.data.custom_font if self.settings.data.use_custom_font else "System Default"
        self._preferences_window.custom_font_picker_row.set_subtitle(font_label)
        self._preferences_window.line_numbers_row.set_active(self.settings.data.show_line_numbers)
        self._preferences_window.wrap_row.set_active(self.settings.data.wrap_text)
        self._preferences_window.ruler_row.set_active(self.settings.data.show_ruler)
        self._rebuild_preferences_folders()
        self._sync_appearance_preview()
        self._syncing_preferences = False

    def _rebuild_preferences_folders(self) -> None:
        if self._preferences_window is None:
            return

        box = self._preferences_window.folders_box
        child = box.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            box.remove(child)
            child = next_child

        active_folder = self.repository.notes_dir
        can_remove = len(self.settings.note_folders) > 1
        for index, folder in enumerate(self.settings.note_folders):
            row = Adw.ActionRow(title=folder.name or str(folder))
            row.set_subtitle(str(folder))
            row.set_activatable(True)
            row.connect("activated", self._on_preferences_use_folder, folder)

            icon = Gtk.Image.new_from_icon_name("folder-symbolic")
            icon.add_css_class(folder_color_css_class(self.settings.get_folder_color(folder)))
            row.add_prefix(icon)

            if folder == active_folder:
                current = Gtk.Label(label="Current")
                current.add_css_class("caption")
                current.add_css_class("dim-label")
                row.add_suffix(current)

            if can_remove:
                remove_button = Gtk.Button(icon_name="user-trash-symbolic")
                remove_button.add_css_class("flat")
                remove_button.set_tooltip_text("Remove Folder")
                remove_button.connect("clicked", self._on_preferences_remove_folder, folder)
                row.add_suffix(remove_button)

            box.append(row)

    def _present_folder_edit_window(self, folder: Path) -> None:
        if self._folder_edit_window is not None:
            self._folder_edit_window.close()

        window = Adw.Window(transient_for=self, modal=True)
        window.add_css_class("papertrail-dialog")
        window.set_title("Edit Folder")
        window.set_default_size(420, 320)
        window.set_hide_on_close(True)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18)
        content.set_margin_top(18)
        content.set_margin_bottom(18)
        content.set_margin_start(18)
        content.set_margin_end(18)

        name_row = Adw.EntryRow(title="Name")
        name_row.set_text(folder.name)

        color_label = Gtk.Label(label="Color", xalign=0)
        color_label.add_css_class("heading")

        color_box = Gtk.FlowBox()
        color_box.set_selection_mode(Gtk.SelectionMode.NONE)
        color_box.set_max_children_per_line(6)
        color_box.set_min_children_per_line(6)
        color_box.set_column_spacing(8)
        color_box.set_row_spacing(8)
        selected_color = self.settings.get_folder_color(folder)

        custom_color_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        custom_color_row.set_halign(Gtk.Align.START)

        color_buttons: list[Gtk.ToggleButton] = []
        group_leader: Gtk.ToggleButton | None = None
        for index, _preset in enumerate(PRESET_FOLDER_COLORS):
            button = Gtk.ToggleButton()
            button.add_css_class("flat")
            button.add_css_class("folder-color-picker")
            button.add_css_class(f"folder-color-{index}")
            button.set_size_request(28, 28)
            if group_leader is None:
                group_leader = button
            else:
                button.set_group(group_leader)
            button.set_active(selected_color == f"preset-{index}")
            button.connect("toggled", self._on_folder_color_preset_toggled, f"preset-{index}", window)
            color_buttons.append(button)
            color_box.insert(button, -1)

        custom_color_label = Gtk.Label(label="Custom Color", xalign=0)
        custom_color_label.add_css_class("heading")

        color_dialog = Gtk.ColorDialog()
        color_dialog.set_title("Select Folder Color")
        color_dialog.set_modal(True)
        custom_color_button = Gtk.ToggleButton()
        custom_color_button.add_css_class("flat")
        custom_color_button.add_css_class("folder-color-picker")
        custom_color_button.set_size_request(56, 28)
        custom_color_button.connect(
            "clicked",
            self._on_folder_custom_color_clicked,
            color_dialog,
            color_buttons,
            window,
        )
        self._sync_folder_custom_color_button(
            custom_color_button,
            self._rgba_to_hex(self._folder_color_rgba(selected_color)),
            is_custom_folder_color(selected_color),
        )
        custom_color_row.append(custom_color_button)
        window.custom_color_button = custom_color_button
        window.custom_color_value = self._rgba_to_hex(self._folder_color_rgba(selected_color))
        window.folder_color_selection = selected_color

        cancel_button = Gtk.Button(label="Cancel")
        cancel_button.add_css_class("flat")
        cancel_button.connect("clicked", lambda *_args: window.close())

        save_button = Gtk.Button(label="Save")
        save_button.add_css_class("suggested-action")
        save_button.connect(
            "clicked",
            self._on_folder_edit_save_clicked,
            folder,
            name_row,
            color_buttons,
            custom_color_button,
            window,
        )

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        actions.set_halign(Gtk.Align.END)
        actions.append(cancel_button)
        actions.append(save_button)

        content.append(name_row)
        content.append(color_label)
        content.append(color_box)
        content.append(custom_color_label)
        content.append(custom_color_row)
        content.append(actions)
        window.set_content(content)
        self._folder_edit_window = window
        window.present()

    def _on_folder_color_preset_toggled(
        self,
        button: Gtk.ToggleButton,
        color_token: str,
        window: Adw.Window,
    ) -> None:
        if button.get_active():
            window.folder_color_selection = color_token
            custom_button = getattr(window, "custom_color_button", None)
            if isinstance(custom_button, Gtk.ToggleButton):
                self._sync_folder_custom_color_button(
                    custom_button,
                    getattr(window, "custom_color_value", self._rgba_to_hex(self._folder_color_rgba(color_token))),
                    False,
                )

    def _on_folder_custom_color_clicked(
        self,
        button: Gtk.ToggleButton,
        color_dialog: Gtk.ColorDialog,
        color_buttons: list[Gtk.ToggleButton],
        window: Adw.Window,
    ) -> None:
        button.set_active(is_custom_folder_color(window.folder_color_selection))
        initial = self._folder_color_rgba(getattr(window, "custom_color_value", window.folder_color_selection))
        color_dialog.choose_rgba(self, initial, None, self._on_folder_custom_color_chosen, button, color_buttons, window)

    def _on_folder_custom_color_chosen(
        self,
        dialog: Gtk.ColorDialog,
        result: Gio.AsyncResult,
        button: Gtk.ToggleButton,
        color_buttons: list[Gtk.ToggleButton],
        window: Adw.Window,
    ) -> None:
        try:
            rgba = dialog.choose_rgba_finish(result)
        except GLib.Error:
            return

        window.custom_color_value = self._rgba_to_hex(rgba)
        window.folder_color_selection = window.custom_color_value
        for color_button in color_buttons:
            if color_button.get_active():
                color_button.set_active(False)
        self._sync_folder_custom_color_button(button, window.custom_color_value, True)

    def _sync_folder_custom_color_button(
        self,
        button: Gtk.ToggleButton,
        color_token: str,
        selected: bool,
    ) -> None:
        if is_custom_folder_color(color_token):
            self._refresh_folder_color_css({color_token})
        previous_class = getattr(button, "folder_color_class", "")
        if previous_class:
            button.remove_css_class(previous_class)
        color_class = folder_color_css_class(color_token)
        button.add_css_class(color_class)
        button.set_active(selected)
        button.folder_color_class = color_class

    def _normalise_folder_name(self, value: str) -> str:
        cleaned = value.strip().replace("/", "-").replace("\\", "-")
        cleaned = "".join(ch for ch in cleaned if ch not in '<>:"|?*').strip().rstrip(".")
        return cleaned or "Untitled Folder"

    def _on_folder_edit_save_clicked(
        self,
        _button: Gtk.Button,
        folder: Path,
        name_row: Adw.EntryRow,
        color_buttons: list[Gtk.ToggleButton],
        custom_color_button: Gtk.ToggleButton,
        window: Adw.Window,
    ) -> None:
        color_token = getattr(window, "folder_color_selection", "preset-0")
        if not is_custom_folder_color(color_token):
            color_token = next((f"preset-{index}" for index, btn in enumerate(color_buttons) if btn.get_active()), color_token)
        else:
            color_token = getattr(window, "custom_color_value", color_token)
        target_name = self._normalise_folder_name(name_row.get_text())
        target_path = folder.parent / target_name

        self._flush_pending_save()
        if target_path != folder:
            if target_path.exists():
                self._show_error_dialog(
                    "Could not rename folder",
                    f"A folder named “{target_name}” already exists in this location.",
                )
                return
            try:
                folder.rename(target_path)
            except OSError as error:
                self._show_error_dialog(
                    "Could not rename folder",
                    str(error) or "The folder could not be renamed.",
                )
                return
            self.settings.rename_folder(folder, target_path)
            if self.repository.notes_dir == folder:
                self.repository.set_notes_dir(target_path)
        else:
            target_path = folder

        self.settings.set_folder_color(target_path, color_token)
        self.current_note = None
        self._refresh_notes()
        self._show_empty_state()
        self._sync_preferences_window()
        window.close()
        self._folder_edit_window = None

    def _on_preferences_use_folder(self, _widget, folder: Path) -> None:
        self._set_active_folder(folder)

    def _on_preferences_remove_folder(self, _widget, folder: Path) -> None:
        self._close_folder(folder)

    def _on_preferences_line_numbers_changed(self, row: Adw.SwitchRow, *_args) -> None:
        if self._syncing_preferences:
            return
        value = row.get_active()
        action = self.lookup_action("toggle-line-numbers")
        if isinstance(action, Gio.SimpleAction):
            action.change_state(GLib.Variant.new_boolean(value))

    def _on_preferences_wrap_changed(self, row: Adw.SwitchRow, *_args) -> None:
        if self._syncing_preferences:
            return
        value = row.get_active()
        action = self.lookup_action("toggle-wrap")
        if isinstance(action, Gio.SimpleAction):
            action.change_state(GLib.Variant.new_boolean(value))

    def _on_preferences_ruler_changed(self, row: Adw.SwitchRow, *_args) -> None:
        if self._syncing_preferences:
            return
        self.settings.data.show_ruler = row.get_active()
        self.settings.save()
        self._apply_ruler_settings()

    def _on_preferences_pick_custom_font(self, *_args) -> None:
        if self._preferences_window is None:
            return
        initial = Pango.FontDescription.from_string(self.settings.data.custom_font)
        self._preferences_window.font_dialog.choose_font(
            self,
            initial,
            None,
            self._on_preferences_font_dialog_finished,
            None,
        )

    def _on_preferences_font_dialog_finished(
        self,
        dialog: Gtk.FontDialog,
        result: Gio.AsyncResult,
        _user_data=None,
    ) -> None:
        if self._syncing_preferences:
            return
        try:
            font_desc = dialog.choose_font_finish(result)
        except GLib.Error:
            return
        font_spec = font_desc.to_string()
        if font_spec == self.settings.data.custom_font:
            return
        self.settings.data.custom_font = font_spec
        self.settings.data.use_custom_font = True
        self.settings.save()
        self._apply_editor_typography()

    def _toggle_info(self, *_args) -> None:
        if self.current_note is None:
            return
        reveal = not self.info_revealer.get_reveal_child()
        if reveal:
            self.info_revealer.set_visible(True)
        self.info_revealer.set_reveal_child(reveal)
        self.info_button.set_active(reveal)

    def _on_info_child_revealed(self, revealer: Gtk.Revealer, *_args) -> None:
        if not revealer.get_reveal_child() and not revealer.get_child_revealed():
            revealer.set_visible(False)

    def _on_sidebar_search_changed(self, *_args) -> None:
        selected = self.current_note.path if self.current_note else None
        self._refresh_notes(selected_path=selected)

    def _on_sidebar_search_button_clicked(self, *_args) -> None:
        reveal = not self.sidebar_search_revealer.get_reveal_child()
        self.sidebar_search_revealer.set_reveal_child(reveal)
        if reveal:
            self.sidebar_search_entry.grab_focus()
            self.sidebar_search_entry.select_region(0, -1)

    def _on_sidebar_search_stop(self, *_args) -> None:
        self._close_sidebar_search()

    def _close_sidebar_search(self) -> None:
        if self.sidebar_search_entry.get_text():
            self.sidebar_search_entry.set_text("")
        self.sidebar_search_revealer.set_reveal_child(False)
        self.editor_view.grab_focus()

    def _on_window_escape_shortcut(self, *_args) -> bool:
        if self.sidebar_search_revealer.get_reveal_child():
            self._close_sidebar_search()
            return True
        return False

    def _on_window_key_pressed(
        self,
        _controller: Gtk.EventControllerKey,
        keyval: int,
        _keycode: int,
        _state: Gdk.ModifierType,
    ) -> bool:
        if keyval == Gdk.KEY_Escape and self.sidebar_search_revealer.get_reveal_child():
            self._close_sidebar_search()
            return True
        return False

    def _on_row_activated(self, row: NoteRow, *_args) -> None:
        if self.current_note is not None and self.current_note.path == row.note_path:
            return
        note = self._get_note_for_path(row.note_path)
        if note is not None:
            if self._show_all_folders or note.path.parent != self.repository.notes_dir:
                self._set_active_folder(note.path.parent)
            self._show_note(note)

    def _on_sidebar_row_delete_requested(self, row: NoteRow, *_args) -> None:
        self._confirm_delete_note(row.note_path)

    def _on_sidebar_row_open_folder_requested(self, row: NoteRow, *_args) -> None:
        self._open_note_folder(row.note_path)

    def _on_sidebar_row_move_to_folder_requested(self, row: NoteRow, folder_path: str) -> None:
        self._move_note_by_path(row.note_path, Path(folder_path))

    def _folder_color_rgba(self, color_token: str) -> Gdk.RGBA:
        rgba = Gdk.RGBA()
        source = color_token if color_token.startswith("#") else PRESET_FOLDER_COLORS[int(color_token.removeprefix("preset-"))]
        rgba.parse(source)
        return rgba

    def _on_buffer_changed(self, *_args) -> None:
        if self._loading_note or self.current_note is None:
            return

        self._update_editor_header_from_buffer()
        self._update_info_panel(self.current_note, live_text=self._get_editor_text())
        self._refresh_search_status()
        if self._save_source_id is not None:
            GLib.source_remove(self._save_source_id)
        self._save_source_id = GLib.timeout_add(350, self._save_current_note)

    def _update_editor_header_from_buffer(self) -> None:
        buffer = self.editor_view.get_buffer()
        text = buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)
        first_line = text.splitlines()[0].strip() if text.splitlines() else ""
        title = first_line or "Untitled Note"
        self._set_title_label(title[:120])

    def _save_current_note(self) -> bool:
        self._save_source_id = None
        if self.current_note is None:
            return GLib.SOURCE_REMOVE

        buffer = self.editor_view.get_buffer()
        text = buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)
        saved = self.repository.save_note(self.current_note.path, text)
        self.current_note = saved
        self._replace_cached_note(saved)
        self._apply_note_language(saved)
        self._set_title_label(saved.title)
        self._set_filename_label(saved.path.name)
        self._update_info_panel(saved)
        self._refresh_search_status()
        query = self.sidebar_search_entry.get_text().strip().casefold()
        if query and query not in saved.title.casefold() and query not in saved.body.casefold():
            self._refresh_notes(selected_path=saved.path)
        else:
            self._update_sidebar_row(saved)
        return GLib.SOURCE_REMOVE

    def _set_title_label(self, title: str) -> None:
        self.editor_title_label.set_text(title)

    def _set_filename_label(self, filename: str) -> None:
        self.editor_filename_label.set_text(filename)

    def _get_editor_text(self) -> str:
        buffer = self.editor_view.get_buffer()
        return buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)

    def _clear_info_panel(self) -> None:
        self._syncing_info_fields = True
        self.info_title_row.set_subtitle("")
        self.info_name_row.set_subtitle("")
        self.info_location_row.set_subtitle("")
        self.info_type_row.set_subtitle("")
        self._syncing_info_fields = False
        self.language_search_entry.set_text("")
        self._rebuild_language_list()

    def _update_info_panel(self, note: NoteRecord, live_text: str | None = None) -> None:
        language = self.editor_view.get_buffer().get_language()
        language_name = language.get_name() if language is not None else "Plain Text"

        title = note.title
        if live_text is not None:
            lines = live_text.splitlines()
            title = lines[0].strip() if lines else ""
            title = title or "Untitled Note"

        self._syncing_info_fields = True
        self.info_title_row.set_subtitle(title)
        self.info_name_row.set_subtitle(note.path.name)
        self.info_location_row.set_subtitle(str(note.path.parent))
        self.info_type_row.set_subtitle(language_name)
        self._syncing_info_fields = False

    def _apply_note_language(self, note: NoteRecord) -> None:
        buffer = self.editor_view.get_buffer()
        override_id = self.settings.get_language_override(note.path)
        language = None
        if override_id:
            language = self._language_manager.get_language(override_id)
        if language is None:
            language = self._language_manager.guess_language(note.path.name, None)
        buffer.set_language(language)
        buffer.set_highlight_syntax(language is not None)

    def _get_note_language_label(self, note: NoteRecord) -> str:
        override_id = self.settings.get_language_override(note.path)
        language = None
        if override_id:
            language = self._language_manager.get_language(override_id)
        if language is None:
            language = self._language_manager.guess_language(note.path.name, None)
        if language is None:
            return "Text"
        return language.get_name()

    def _sync_language_selection(self, note: NoteRecord | None) -> None:
        self._syncing_language_list = True
        selected_id = None
        if note is not None:
            override_id = self.settings.get_language_override(note.path)
            if override_id:
                selected_id = override_id

        child = self.language_listbox.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            child.remove_css_class("selected")
            language_id = getattr(child, "language_id", None)
            if selected_id is None:
                if language_id is None:
                    child.add_css_class("selected")
            elif language_id == selected_id:
                child.add_css_class("selected")
            child = next_child
        self._syncing_language_list = False

    def _on_info_type_activated(self, *_args) -> None:
        if self.current_note is None:
            return
        self.language_popout_button.popup()

    def _on_info_type_clicked(self, *_args) -> None:
        self._on_info_type_activated()

    def _on_language_popover_visible_changed(self, popover: Gtk.Popover, *_args) -> None:
        if not popover.get_visible():
            return
        self.language_search_entry.grab_focus()
        self.language_search_entry.select_region(0, -1)

    def _rebuild_language_list(self) -> None:
        query = self.language_search_entry.get_text().strip().casefold()
        child = self.language_listbox.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            self.language_listbox.remove(child)
            child = next_child

        for label, language_id in self._language_options:
            haystack = label.casefold()
            if language_id is not None:
                haystack = f"{haystack} {language_id.casefold()}"
            if query and query not in haystack:
                continue

            row = Gtk.ListBoxRow()
            row.language_id = language_id

            text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
            text.set_margin_top(8)
            text.set_margin_bottom(8)
            text.set_margin_start(10)
            text.set_margin_end(10)

            title = Gtk.Label(xalign=0)
            title.set_text(label)
            if language_id is None:
                title.add_css_class("heading")

            subtitle = Gtk.Label(xalign=0)
            subtitle.add_css_class("caption")
            subtitle.add_css_class("dim-label")
            subtitle.set_text(language_id or "Use filename-based detection")

            text.append(title)
            text.append(subtitle)
            row.set_child(text)
            self.language_listbox.append(row)

        self._sync_language_selection(self.current_note)

    def _update_style_scheme(self) -> None:
        scheme_id = self._effective_style_scheme_id()
        scheme = self._style_scheme_manager.get_scheme(scheme_id)
        if scheme is None:
            scheme = self._style_scheme_manager.get_scheme("classic")
        self.editor_view.get_buffer().set_style_scheme(scheme)
        self._apply_scheme_window_recolor(scheme)
        self._sync_scheme_previews()

    def _apply_scheme_window_recolor(self, scheme: GtkSource.StyleScheme | None) -> None:
        if scheme is None:
            self._scheme_recolor_provider.load_from_data(b"")
            return

        if scheme.get_name().startswith("Adwaita"):
            self._scheme_recolor_provider.load_from_data(b"")
            return

        text_style = scheme.get_style("text")
        selection_style = scheme.get_style("selection")
        current_line_style = scheme.get_style("current-line")

        text_bg = self._parse_style_rgba(text_style, "background") or Gdk.RGBA(1, 1, 1, 1)
        text_fg = self._parse_style_rgba(text_style, "foreground") or Gdk.RGBA(0.18, 0.18, 0.2, 1)
        current_bg = self._parse_style_rgba(current_line_style, "background") or text_bg
        selection_rgba = self._parse_style_rgba(selection_style, "background")
        is_dark = self._scheme_is_dark(scheme, text_bg)
        white = Gdk.RGBA(1, 1, 1, 1)
        black = Gdk.RGBA(0, 0, 0, 1)
        alt = white if is_dark else black

        window_bg = self._get_metadata_rgba(scheme, "window_bg_color")
        if window_bg is None:
            if is_dark:
                window_bg = text_bg
            else:
                window_bg = self._mix_rgba(text_bg, text_fg, 0.03)

        window_fg_rgba = self._get_metadata_rgba(scheme, "window_fg_color")
        if window_fg_rgba is None:
            window_fg_rgba = text_fg if self._parse_style_rgba(text_style, "foreground") else self._mix_rgba(text_bg, alt, 0.025 if not is_dark else 0.05)

        sidebar_bg = self._get_metadata_rgba(scheme, "sidebar_bg_color")
        if sidebar_bg is None:
            sidebar_bg = self._parse_style_rgba(scheme.get_style("line-numbers"), "background") or text_bg

        headerbar_bg_rgba = self._get_metadata_rgba(scheme, "headerbar_bg_color")
        if headerbar_bg_rgba is None:
            headerbar_bg_rgba = self._mix_rgba(text_bg, white, 0.07 if is_dark else 0.25)

        headerbar_fg_rgba = self._get_metadata_rgba(scheme, "headerbar_fg_color")
        if headerbar_fg_rgba is None:
            headerbar_fg_rgba = text_fg if self._parse_style_rgba(text_style, "foreground") else self._mix_rgba(text_bg, alt, 0.025 if not is_dark else 0.05)

        popover_bg = self._get_metadata_rgba(scheme, "popover_bg_color") or self._mix_rgba(text_bg, white, 0.07 if is_dark else 0.25)
        popover_fg = self._get_metadata_rgba(scheme, "popover_fg_color") or window_fg_rgba
        dialog_bg = self._mix_rgba(text_bg, white, 0.07) if is_dark else text_bg
        view_bg = self._mix_rgba(text_bg, black, 0.1) if is_dark else self._mix_rgba(text_bg, white, 0.3)
        accent_rgba = self._get_metadata_rgba(scheme, "accent_bg_color") or selection_rgba or text_fg
        accent_fg_rgba = self._get_metadata_rgba(scheme, "accent_fg_color") or self._parse_style_rgba(selection_style, "foreground") or white

        base_bg = self._rgba_to_css(text_bg)
        base_fg = self._rgba_to_css(text_fg)
        current_bg_css = self._rgba_to_css(current_bg)
        window_bg_css = self._rgba_to_css(window_bg)
        window_fg = self._rgba_to_css(window_fg_rgba)
        headerbar_bg = self._rgba_to_css(headerbar_bg_rgba)
        headerbar_fg = self._rgba_to_css(headerbar_fg_rgba)
        sidebar_bg_css = self._rgba_to_css(sidebar_bg)
        folder_column_bg_css = self._rgba_to_css(self._mix_rgba(sidebar_bg, black, 0.06))
        popover_bg_css = self._rgba_to_css(popover_bg)
        popover_fg_css = self._rgba_to_css(popover_fg)
        dialog_bg_css = self._rgba_to_css(dialog_bg)
        view_bg_css = self._rgba_to_css(view_bg)
        accent_bg = self._rgba_to_css(accent_rgba)
        accent_fg = self._rgba_to_css(accent_fg_rgba)
        note_fg_css = window_fg
        note_dim_fg_rgba = self._mix_rgba(window_fg_rgba, text_bg, 0.3)
        note_dim_fg_css = self._rgba_to_css(note_dim_fg_rgba)
        note_active_fg_rgba = self._parse_style_rgba(selection_style, "foreground") or self._best_contrast_rgba(accent_rgba)
        note_active_fg_css = self._rgba_to_css(note_active_fg_rgba)
        note_pill_bg_rgba = self._mix_rgba(sidebar_bg, note_dim_fg_rgba, 0.12)
        note_pill_fg_rgba = window_fg_rgba
        note_pill_border_rgba = self._mix_rgba(sidebar_bg, window_fg_rgba, 0.18)
        note_pill_active_bg_rgba = self._mix_rgba(accent_rgba, note_active_fg_rgba, 0.22)
        note_pill_active_fg_rgba = self._best_contrast_rgba(note_pill_active_bg_rgba)
        note_pill_active_border_rgba = self._mix_rgba(note_pill_active_bg_rgba, note_pill_active_fg_rgba, 0.35)
        note_pill_bg_css = self._rgba_to_css(note_pill_bg_rgba)
        note_pill_fg_css = self._rgba_to_css(note_pill_fg_rgba)
        note_pill_border_css = self._rgba_to_css(note_pill_border_rgba)
        note_pill_active_bg_css = self._rgba_to_css(note_pill_active_bg_rgba)
        note_pill_active_fg_css = self._rgba_to_css(note_pill_active_fg_rgba)
        note_pill_active_border_css = self._rgba_to_css(note_pill_active_border_rgba)

        css = f"""
window,
popover,
popover.context-menu {{
  color: {window_fg};
  --window-bg-color: {popover_bg_css};
  --window-fg-color: {popover_fg_css};
  --view-bg-color: {view_bg_css};
  --popover-bg-color: {popover_bg_css};
  --dialog-bg-color: {dialog_bg_css};
  --card-bg-color: {popover_bg_css};
  --headerbar-bg-color: {popover_bg_css};
  --accent-bg-color: {accent_bg};
  --accent-fg-color: {accent_fg};
  --papertrail-shell-bg: {window_bg_css};
  --papertrail-panel-bg: {dialog_bg_css};
  --papertrail-sidebar-surface: {sidebar_bg_css};
  --papertrail-sidebar-hover: color-mix(in srgb, {accent_bg} 12%, {sidebar_bg_css});
  --papertrail-note-active-bg: {accent_bg};
  --papertrail-note-fg: {note_fg_css};
  --papertrail-note-dim-fg: {note_dim_fg_css};
  --papertrail-note-active-fg: {note_active_fg_css};
  --papertrail-note-pill-bg: {note_pill_bg_css};
  --papertrail-note-pill-fg: {note_pill_fg_css};
  --papertrail-note-pill-border: {note_pill_border_css};
  --papertrail-note-pill-active-bg: {note_pill_active_bg_css};
  --papertrail-note-pill-active-fg: {note_pill_active_fg_css};
  --papertrail-note-pill-active-border: {note_pill_active_border_css};
  --papertrail-menu-surface: {popover_bg_css};
  --papertrail-input-surface: {view_bg_css};
  --papertrail-popover-surface: {popover_bg_css};
  --papertrail-scheme-check-bg: {accent_bg};
  --papertrail-scheme-check-fg: {accent_fg};
}}

window {{
  background: {window_bg_css};
  background-image: none;
}}

window.papertrail-dialog {{
  background-image: none;
  background-color: {dialog_bg_css};
}}

window.papertrail-dialog.papertrail-preferences preferencespage,
window.papertrail-dialog.papertrail-preferences .papertrail-preferences-page,
window.papertrail-dialog.papertrail-preferences .papertrail-preferences-page > scrolledwindow,
window.papertrail-dialog.papertrail-preferences .papertrail-preferences-page > scrolledwindow > viewport,
window.papertrail-dialog.papertrail-preferences preferencespage > scrolledwindow > viewport {{
  background-image: none;
  background-color: {dialog_bg_css};
}}

window headerbar {{
  color: {headerbar_fg};
  background-color: {headerbar_bg};
}}

window headerbar label,
window headerbar button,
window headerbar image {{
  color: {headerbar_fg};
}}

window.papertrail-window .info-panel {{
  background-color: {dialog_bg_css};
}}

window.papertrail-window .sidebar-root,
window.papertrail-window .sidebar-header,
window.papertrail-window .sidebar-body,
window.papertrail-window .sidebar-search-box,
window.papertrail-window .sidebar-notes-scroller,
window.papertrail-window .sidebar-notes-viewport,
window.papertrail-window .sidebar-note-list {{
  background-color: {sidebar_bg_css};
}}

window.papertrail-window .sidebar-folders-column {{
  background-color: {folder_column_bg_css};
  background-image: none;
}}

window.papertrail-window .sidebar-folders-scroller,
window.papertrail-window .sidebar-folders-scroller > viewport,
window.papertrail-window .sidebar-folders-viewport,
window.papertrail-window .sidebar-folder-list,
window.papertrail-window .sidebar-folder-add-box {{
  background-color: transparent;
  background-image: none;
}}

window.papertrail-window .sidebar-search-entry {{
  background-color: color-mix(in srgb, {sidebar_bg_css} 72%, {window_fg} 6%);
  color: {window_fg};
}}

window.papertrail-window .sidebar-search-entry image,
window.papertrail-window .sidebar-search-entry placeholder {{
  color: color-mix(in srgb, {window_fg} 60%, transparent);
}}

popover.papertrail-menu-popover contents,
popover.papertrail-menu-popover > contents {{
  background-color: {popover_bg_css};
  color: {popover_fg_css};
}}

popover.papertrail-menu-popover .papertrail-menu-surface {{
  background-color: {popover_bg_css};
}}

popover.papertrail-menu-popover .menu-zoom-row label,
popover.papertrail-menu-popover modelbutton,
popover.papertrail-menu-popover button,
popover.papertrail-menu-popover image {{
  color: {popover_fg_css};
}}

window entry,
window searchentry,
window text,
window .entry,
popover.papertrail-popover entry,
popover.papertrail-popover searchentry,
popover.papertrail-menu-popover entry,
popover.papertrail-menu-popover searchentry {{
  color: {window_fg};
}}

window entry,
window searchentry,
popover.papertrail-popover entry,
popover.papertrail-popover searchentry,
popover.papertrail-menu-popover entry,
popover.papertrail-menu-popover searchentry {{
  background-color: {view_bg_css};
}}

popover.papertrail-popover contents,
popover.papertrail-popover > contents,
popover.papertrail-popover > arrow,
popover.papertrail-popover > arrow > border,
popover.papertrail-menu-popover > arrow,
popover.papertrail-menu-popover > arrow > border,
popover.context-menu > contents,
popover.context-menu > menu,
popover.context-menu menu,
popover.context-menu > arrow,
popover.context-menu > arrow > border,
textview.editor-view window.popup,
textview.editor-view window.popup popover.menu > contents,
textview.editor-view window.popup popover.menu > arrow,
textview.editor-view window.popup popover.menu > arrow > border {{
  background-color: {popover_bg_css};
  color: {popover_fg_css};
}}

popover.papertrail-emoji-chooser,
popover.papertrail-emoji-chooser > contents,
popover.papertrail-emoji-chooser box,
popover.papertrail-emoji-chooser scrolledwindow,
popover.papertrail-emoji-chooser viewport,
popover.papertrail-emoji-chooser flowbox,
popover.papertrail-emoji-chooser flowboxchild {{
  background-color: {popover_bg_css};
  background-image: none;
  color: {popover_fg_css};
}}

popover.papertrail-emoji-chooser .emoji-searchbar,
popover.papertrail-emoji-chooser .emoji-toolbar {{
  background-color: {popover_bg_css};
  color: {popover_fg_css};
}}

popover.papertrail-emoji-chooser label,
popover.papertrail-emoji-chooser .emoji-section,
popover.papertrail-emoji-chooser .emoji-toolbar button,
popover.papertrail-emoji-chooser .emoji-toolbar image {{
  color: {popover_fg_css};
}}

popover.papertrail-emoji-chooser .emoji-toolbar button,
popover.papertrail-emoji-chooser .emoji-section {{
  background: transparent;
  background-image: none;
  box-shadow: none;
}}

popover.papertrail-emoji-chooser .emoji-toolbar button:hover,
popover.papertrail-emoji-chooser .emoji-toolbar button:checked,
popover.papertrail-emoji-chooser .emoji-section:hover,
popover.papertrail-emoji-chooser .emoji-section:checked {{
  background-color: color-mix(in srgb, {accent_bg} 16%, transparent);
}}

popover.context-menu modelbutton,
popover.context-menu modelbutton label,
popover.context-menu accelerator,
popover.context-menu button,
popover.context-menu button label,
popover.context-menu image,
popover.context-menu check,
popover.context-menu radio,
textview.editor-view window.popup button.model,
textview.editor-view window.popup button.model label,
textview.editor-view window.popup accelerator,
textview.editor-view window.popup image,
textview.editor-view window.popup check,
textview.editor-view window.popup radio {{
  color: {popover_fg_css};
}}

popover.context-menu separator,
textview.editor-view window.popup separator {{
  background-color: color-mix(in srgb, {popover_fg_css} 14%, transparent);
}}

.scheme-preview-button:checked .scheme-preview-frame {{
  box-shadow:
    0 0 0 2px {accent_bg},
    0 1px 3px color-mix(in srgb, {base_fg} 18%, transparent);
}}

.scheme-preview-frame {{
  box-shadow:
    0 0 0 1px color-mix(in srgb, {base_fg} 8%, transparent),
    0 1px 3px color-mix(in srgb, {base_fg} 10%, transparent);
  background: color-mix(in srgb, {current_bg_css} 40%, {base_bg});
}}

.scheme-preview-button:checked check,
.scheme-preview-button:checked radio {{
  background-color: {accent_bg};
  color: {accent_fg};
}}
""".encode("utf-8")
        self._scheme_recolor_provider.load_from_data(css)

    def _style_color(self, style: GtkSource.Style | None, prop: str) -> str | None:
        if style is None:
            return None
        try:
            value = style.get_property(prop)
        except TypeError:
            return None
        if not value:
            return None
        return str(value)

    def _parse_style_rgba(self, style: GtkSource.Style | None, prop: str) -> Gdk.RGBA | None:
        value = self._style_color(style, prop)
        if not value:
            return None
        rgba = Gdk.RGBA()
        if rgba.parse(value) and rgba.alpha >= 0.1:
            return rgba
        return None

    def _get_metadata_rgba(self, scheme: GtkSource.StyleScheme, key: str) -> Gdk.RGBA | None:
        value = scheme.get_metadata(key)
        if not value:
            return None
        rgba = Gdk.RGBA()
        if rgba.parse(value):
            return rgba
        return None

    def _rgba_to_css(self, rgba: Gdk.RGBA) -> str:
        opaque = Gdk.RGBA(rgba.red, rgba.green, rgba.blue, 1.0)
        return opaque.to_string()

    def _rgba_to_hex(self, rgba: Gdk.RGBA) -> str:
        return "#{:02x}{:02x}{:02x}".format(
            round(rgba.red * 255),
            round(rgba.green * 255),
            round(rgba.blue * 255),
        )

    def _mix_rgba(self, a: Gdk.RGBA, b: Gdk.RGBA, level: float) -> Gdk.RGBA:
        return Gdk.RGBA(
            red=((1 - level) * a.red) + (level * b.red),
            green=((1 - level) * a.green) + (level * b.green),
            blue=((1 - level) * a.blue) + (level * b.blue),
            alpha=1.0,
        )

    def _best_contrast_rgba(self, bg: Gdk.RGBA) -> Gdk.RGBA:
        white = Gdk.RGBA(1, 1, 1, 1)
        black = Gdk.RGBA(0, 0, 0, 1)
        white_contrast = self._contrast_ratio(bg, white)
        black_contrast = self._contrast_ratio(bg, black)
        return white if white_contrast >= black_contrast else black

    def _contrast_ratio(self, a: Gdk.RGBA, b: Gdk.RGBA) -> float:
        a_luminance = self._relative_luminance(a)
        b_luminance = self._relative_luminance(b)
        lighter = max(a_luminance, b_luminance)
        darker = min(a_luminance, b_luminance)
        return (lighter + 0.05) / (darker + 0.05)

    def _relative_luminance(self, color: Gdk.RGBA) -> float:
        def channel(v: float) -> float:
            return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4

        r = channel(color.red)
        g = channel(color.green)
        b = channel(color.blue)
        return (0.2126 * r) + (0.7152 * g) + (0.0722 * b)

    def _scheme_is_dark(self, scheme: GtkSource.StyleScheme, text_bg: Gdk.RGBA) -> bool:
        scheme_id = scheme.get_id() or ""
        variant = scheme.get_metadata("variant")
        if variant == "light":
            return False
        if variant == "dark":
            return True
        if "-dark" in scheme_id:
            return True
        r = text_bg.red * 255.0
        g = text_bg.green * 255.0
        b = text_bg.blue * 255.0
        hsp = math.sqrt((0.299 * (r * r)) + (0.587 * (g * g)) + (0.114 * (b * b)))
        return hsp <= 127.5

    def _effective_style_scheme_id(self) -> str:
        scheme_id = self._normalized_style_scheme_id(self.settings.data.editor_style_scheme)
        scheme = self._style_scheme_manager.get_scheme(scheme_id)
        if scheme is None:
            return "Adwaita-dark" if self._style_manager.get_dark() else "Adwaita"

        if self._style_manager.get_dark():
            dark_variant = scheme.get_metadata("dark-variant")
            if dark_variant and self._style_scheme_manager.get_scheme(dark_variant) is not None:
                return dark_variant
        else:
            light_variant = scheme.get_metadata("light-variant")
            if light_variant and self._style_scheme_manager.get_scheme(light_variant) is not None:
                return light_variant

        return scheme_id

    def _normalized_style_scheme_id(self, scheme_id: str | None) -> str:
        normalized = STYLE_SCHEME_ALIASES.get(scheme_id or "", scheme_id or "Adwaita")
        if not self._style_manager.get_dark() and normalized == "oblivion":
            normalized = "tango"
        allowed_ids = {
            scheme_id
            for _title, scheme_id in (LIGHT_STYLE_SCHEME_CHOICES + DARK_STYLE_SCHEME_CHOICES)
        }
        if normalized not in allowed_ids:
            return "Adwaita"
        return normalized

    def _current_style_scheme_choices(self) -> list[tuple[str, str]]:
        choices = DARK_STYLE_SCHEME_CHOICES if self._style_manager.get_dark() else LIGHT_STYLE_SCHEME_CHOICES
        available: list[tuple[str, str]] = []
        for title, scheme_id in choices:
            if self._style_scheme_manager.get_scheme(scheme_id) is not None:
                available.append((title, scheme_id))
        return available

    def _available_style_schemes(self) -> list[tuple[str, str]]:
        return self._current_style_scheme_choices()

    def _build_scheme_preview_grid(self) -> Gtk.FlowBox:
        flow = Gtk.FlowBox()
        flow.set_selection_mode(Gtk.SelectionMode.NONE)
        flow.set_max_children_per_line(4)
        flow.set_min_children_per_line(2)
        flow.set_valign(Gtk.Align.START)
        flow.add_css_class("scheme-flow")
        self._scheme_preview_buttons = {}
        self._scheme_preview_frames = {}
        self._scheme_preview_checks = {}

        for title, scheme_id in self._available_style_schemes():
            button = Gtk.ToggleButton()
            button.set_tooltip_text(title)
            button.add_css_class("flat")
            button.add_css_class("scheme-preview-button")
            button.connect("clicked", self._on_scheme_preview_clicked, scheme_id)

            frame = Gtk.Frame()
            frame.add_css_class("scheme-preview-frame")

            preview = self._create_scheme_preview_widget(scheme_id)
            frame.set_child(preview)

            overlay = Gtk.Overlay()
            overlay.set_child(frame)

            check = Gtk.Image.new_from_icon_name("object-select-symbolic")
            check.add_css_class("scheme-preview-check")
            check.set_pixel_size(10)
            check.set_halign(Gtk.Align.END)
            check.set_valign(Gtk.Align.END)
            check.set_margin_end(5)
            check.set_margin_bottom(5)
            check.set_visible(False)
            overlay.add_overlay(check)

            button.set_child(overlay)
            self._scheme_preview_buttons[scheme_id] = button
            self._scheme_preview_frames[scheme_id] = frame
            self._scheme_preview_checks[scheme_id] = check
            flow.insert(button, -1)

        self._sync_scheme_previews()
        return flow

    def _rebuild_scheme_preview_grid(self) -> None:
        if self._preferences_window is None:
            return

        old_flow = getattr(self._preferences_window, "scheme_flow", None)
        scheme_holder = getattr(self._preferences_window, "scheme_holder", None)
        if old_flow is None or scheme_holder is None:
            return

        scheme_holder.remove(old_flow)
        new_flow = self._build_scheme_preview_grid()
        scheme_holder.append(new_flow)
        self._preferences_window.scheme_flow = new_flow

    def _create_scheme_preview_widget(self, scheme_id: str) -> Gtk.Widget:
        effective_id = self._effective_preview_scheme_id(scheme_id)
        scheme = self._style_scheme_manager.get_scheme(effective_id)
        preview = GtkSource.StyleSchemePreview.new(scheme) if scheme is not None else Gtk.Box()
        preview.set_size_request(0, 60)
        preview.add_css_class("scheme-preview-view")
        return preview

    def _sync_scheme_previews(self) -> None:
        selected_id = self._normalized_style_scheme_id(self.settings.data.editor_style_scheme)
        for scheme_id, button in self._scheme_preview_buttons.items():
            button.set_active(scheme_id == selected_id)
        for scheme_id, check in self._scheme_preview_checks.items():
            check.set_visible(scheme_id == selected_id)
        for scheme_id, frame in self._scheme_preview_frames.items():
            frame.set_child(self._create_scheme_preview_widget(scheme_id))
        self._sync_appearance_preview()

    def _sync_appearance_preview(self) -> None:
        if self._preferences_window is None:
            return
        preview = getattr(self._preferences_window, "appearance_preview", None)
        if preview is None:
            return
        buffer = preview.get_buffer()
        if not isinstance(buffer, GtkSource.Buffer):
            return
        scheme = self._style_scheme_manager.get_scheme(self._effective_style_scheme_id())
        if scheme is not None:
            buffer.set_style_scheme(scheme)

    def _effective_preview_scheme_id(self, scheme_id: str) -> str:
        scheme = self._style_scheme_manager.get_scheme(scheme_id)
        if scheme is None:
            return scheme_id
        if self._style_manager.get_dark():
            dark_variant = scheme.get_metadata("dark-variant")
            if dark_variant and self._style_scheme_manager.get_scheme(dark_variant) is not None:
                return dark_variant
        return scheme_id

    def _on_scheme_preview_clicked(self, _button: Gtk.ToggleButton, scheme_id: str) -> None:
        current_scheme_id = self._normalized_style_scheme_id(self.settings.data.editor_style_scheme)
        if current_scheme_id == scheme_id:
            self._sync_scheme_previews()
            return
        self.settings.data.editor_style_scheme = scheme_id
        self.settings.save()
        self._update_style_scheme()

    def _on_style_variant_changed(self, *_args) -> None:
        self._update_style_scheme()
        self._rebuild_scheme_preview_grid()

    def _on_language_search_changed(self, *_args) -> None:
        self._rebuild_language_list()

    def _on_language_row_activated(self, _box: Gtk.ListBox, row: Gtk.ListBoxRow) -> None:
        if self._syncing_language_list or self.current_note is None:
            return

        language_id = getattr(row, "language_id", None)
        self.settings.set_language_override(self.current_note.path, language_id)
        self._apply_note_language(self.current_note)
        self._sync_language_selection(self.current_note)
        self._update_info_panel(self.current_note, live_text=self._get_editor_text())
        self._update_sidebar_row(self.current_note)
        self.language_popover.popdown()
        self.editor_view.grab_focus()

    def _on_sidebar_row_rename_submitted(self, row: NoteRow, filename: str) -> None:
        self._rename_note_by_path(row.note_path, filename)

    def _rename_note_by_path(self, note_path: Path, new_filename: str) -> None:
        note = self._get_note_for_path(note_path)
        if self.current_note is not None and self.current_note.path == note_path:
            self._flush_pending_save()
            note = self.current_note
        if note is None or not new_filename or new_filename == note.path.name:
            return

        renamed = self.repository.rename_note(note.path, new_filename)
        self.settings.rename_language_override(note.path, renamed.path)
        self.settings.rename_pinned_note(note.path, renamed.path)
        self._replace_cached_note(renamed, old_path=note.path)
        if self.current_note is not None and self.current_note.path == note.path:
            self.current_note = renamed
            self._apply_note_language(renamed)
            self._set_title_label(renamed.title)
            self._set_filename_label(renamed.path.name)
            self._sync_language_selection(renamed)
            self._update_info_panel(renamed)
        self._refresh_notes(selected_path=renamed.path)

    def _move_note_by_path(self, note_path: Path, destination_folder: Path) -> None:
        destination_folder = destination_folder.expanduser()
        if destination_folder == note_path.parent or not note_path.exists():
            return

        moving_current_note = self.current_note is not None and self.current_note.path == note_path
        if moving_current_note:
            self._flush_pending_save()

        try:
            moved = self.repository.move_note(note_path, destination_folder)
        except OSError as error:
            self._show_error_dialog(
                "Could Not Move Note",
                str(error) or "The note could not be moved to that folder.",
            )
            return

        self.settings.rename_language_override(note_path, moved.path)
        self.settings.rename_pinned_note(note_path, moved.path)
        if destination_folder not in self.settings.note_folders:
            self.settings.add_notes_dir(destination_folder, activate=False)
            self._sync_preferences_window()

        if moving_current_note:
            if destination_folder != self.repository.notes_dir:
                self.settings.set_notes_dir(destination_folder)
                self.repository.set_notes_dir(destination_folder)
                self._sync_preferences_window()
            self.current_note = moved
            self._refresh_notes(selected_path=moved.path)
            self._show_note(moved)
            return

        selected_path = self.current_note.path if self.current_note is not None else None
        self._refresh_notes(selected_path=selected_path)

    def _flush_pending_save(self) -> None:
        if self._save_source_id is None:
            return
        GLib.source_remove(self._save_source_id)
        self._save_source_id = None
        self._save_current_note()

    def _on_editor_search_changed(self, *_args) -> None:
        query = self._get_editor_search_query()
        if self._search_settings is not None:
            self._search_settings.set_search_text(query)
        if self._search_context is not None:
            self._search_context.set_highlight(bool(query))
        self._refresh_search_status(select_first=bool(query))

    def _on_occurrences_count_changed(self, *_args) -> None:
        self._refresh_search_status()

    def _on_search_entry_key_pressed(
        self,
        _controller: Gtk.EventControllerKey,
        keyval: int,
        _keycode: int,
        state: Gdk.ModifierType,
    ) -> bool:
        if keyval == Gdk.KEY_Return:
            if state & Gdk.ModifierType.SHIFT_MASK:
                self._find_previous()
            else:
                self._find_next()
            return True
        return False

    def _on_replace_entry_key_pressed(
        self,
        _controller: Gtk.EventControllerKey,
        keyval: int,
        _keycode: int,
        state: Gdk.ModifierType,
    ) -> bool:
        if keyval == Gdk.KEY_Return:
            if state & Gdk.ModifierType.SHIFT_MASK:
                self._replace_all_matches()
            else:
                self._replace_current_match()
            return True
        return False

    def _find_next(self, *_args) -> None:
        self._find(forward=True)

    def _find_previous(self, *_args) -> None:
        self._find(forward=False)

    def _find(self, forward: bool) -> None:
        match = self._find_match(forward)
        if match is None:
            self.editor_search_status_label.set_text("No matches")
            return

        match_start, match_end = match
        self._select_search_match(match_start, match_end)
        self._refresh_search_status()

    def _find_match(
        self,
        forward: bool,
        *,
        from_start: bool = False,
    ) -> tuple[Gtk.TextIter, Gtk.TextIter] | None:
        query = self._get_editor_search_query()
        if not query or self._search_context is None:
            return None

        buffer = self.editor_view.get_buffer()
        if from_start:
            search_from = buffer.get_start_iter() if forward else buffer.get_end_iter()
        else:
            search_from = buffer.get_iter_at_mark(buffer.get_insert())
            current_match = self._get_selected_search_match()
            if current_match is not None:
                start_iter, end_iter = current_match
                search_from = end_iter if forward else start_iter

        if forward:
            found, match_start, match_end, _wrapped = self._search_context.forward(search_from)
        else:
            found, match_start, match_end, _wrapped = self._search_context.backward(search_from)

        if not found:
            wrap_from = buffer.get_start_iter() if forward else buffer.get_end_iter()
            if forward:
                found, match_start, match_end, _wrapped = self._search_context.forward(wrap_from)
            else:
                found, match_start, match_end, _wrapped = self._search_context.backward(wrap_from)
            if not found:
                return None

        return match_start, match_end

    def _select_search_match(self, match_start: Gtk.TextIter, match_end: Gtk.TextIter) -> None:
        buffer = self.editor_view.get_buffer()
        buffer.select_range(match_start, match_end)
        self.editor_view.scroll_to_iter(match_start, 0.25, False, 0.0, 0.0)

    def _get_selected_search_match(self) -> tuple[Gtk.TextIter, Gtk.TextIter] | None:
        if self._search_context is None:
            return None

        buffer = self.editor_view.get_buffer()
        if not buffer.get_has_selection():
            return None

        match_start, match_end = buffer.get_selection_bounds()
        if self._search_context.get_occurrence_position(match_start, match_end) <= 0:
            return None
        return match_start, match_end

    def _get_editor_search_query(self) -> str:
        return self.editor_search_entry.get_text()

    def _replace_current_match(self, *_args) -> None:
        if self.current_note is None or self._search_context is None:
            return

        query = self._get_editor_search_query()
        if not query:
            return

        current_match = self._get_selected_search_match()
        if current_match is None:
            current_match = self._find_match(forward=True)
        if current_match is None:
            self.editor_search_status_label.set_text("No matches")
            return

        match_start, match_end = current_match

        replace_text = self.editor_replace_entry.get_text()
        self._loading_note = True
        self._search_context.replace(match_start, match_end, replace_text, -1)
        self._loading_note = False
        self._on_buffer_changed()
        self._find_next()

    def _replace_all_matches(self, *_args) -> None:
        if self.current_note is None or self._search_context is None:
            return

        query = self._get_editor_search_query()
        if not query:
            return

        replace_text = self.editor_replace_entry.get_text()
        self._loading_note = True
        replaced = self._search_context.replace_all(replace_text, -1)
        self._loading_note = False
        self._on_buffer_changed()
        self.editor_search_status_label.set_text(
            "1 replacement" if replaced == 1 else f"{replaced} replacements"
        )

    def _refresh_search_status(self, select_first: bool = False) -> None:
        query = self._get_editor_search_query()
        if not query or self._search_context is None:
            self.editor_search_status_label.set_text("")
            return

        count = self._search_context.get_occurrences_count()
        if count < 0:
            self.editor_search_status_label.set_text("Searching...")
            return
        if count == 0:
            self.editor_search_status_label.set_text("No matches")
            return

        buffer = self.editor_view.get_buffer()
        position = 0
        current_match = self._get_selected_search_match()
        if current_match is not None:
            match_start, match_end = current_match
            position = self._search_context.get_occurrence_position(match_start, match_end)

        if select_first and position <= 0:
            match = self._find_match(forward=True, from_start=True)
            if match is not None:
                match_start, match_end = match
                self._select_search_match(match_start, match_end)
                position = self._search_context.get_occurrence_position(match_start, match_end)

        if position > 0:
            self.editor_search_status_label.set_text(f"{position} of {count} matches")
        else:
            self.editor_search_status_label.set_text(
                "1 match" if count == 1 else f"{count} matches"
            )

    def _on_close_request(self, *_args) -> bool:
        self._flush_pending_save()
        self.settings.data.width = self.get_width()
        self.settings.data.height = self.get_height()
        self.settings.save()
        return False
