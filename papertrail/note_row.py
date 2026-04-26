# SPDX-License-Identifier: GPL-3.0-or-later

from __future__ import annotations

from datetime import datetime

from gi.repository import GLib, GObject, Gdk, Gtk, Pango

from .folder_colors import folder_color_css_class
from .note_repository import NoteRecord


def _format_modified(modified_at: datetime) -> str:
    now = datetime.now()
    if modified_at.date() == now.date():
        return modified_at.strftime("%H:%M")
    if modified_at.year == now.year:
        return modified_at.strftime("%b %d")
    return modified_at.strftime("%Y-%m-%d")


def _folder_badge_text(title: str) -> str:
    words = [part for part in title.replace("_", " ").replace("-", " ").split() if part]
    if len(words) >= 2:
        return (words[0][0] + words[1][0]).upper()[:2]
    compact = "".join(ch for ch in title if ch.isalnum())
    return compact[:2].upper() or "F"


class NoteRow(Gtk.Box):
    __gsignals__ = {
        "activated": (GObject.SignalFlags.RUN_FIRST, None, ()),
        "rename-submitted": (GObject.SignalFlags.RUN_FIRST, None, (str,)),
        "pin-toggled": (GObject.SignalFlags.RUN_FIRST, None, (bool,)),
        "open-folder-requested": (GObject.SignalFlags.RUN_FIRST, None, ()),
        "move-to-folder-requested": (GObject.SignalFlags.RUN_FIRST, None, (str,)),
        "delete-requested": (GObject.SignalFlags.RUN_FIRST, None, ()),
    }

    def __init__(self, note: NoteRecord) -> None:
        super().__init__(orientation=Gtk.Orientation.VERTICAL)
        self.add_css_class("note-row")
        self.note_path = note.path
        self._active = False
        self._pinned = False
        self._folder_color_class = ""
        self._move_targets: list[tuple[str, str]] = []

        title_label = Gtk.Label(xalign=0)
        title_label.add_css_class("heading")
        title_label.add_css_class("note-row-title")
        title_label.set_ellipsize(Pango.EllipsizeMode.END)

        pin_icon = Gtk.Image.new_from_icon_name("view-pin-symbolic")
        pin_icon.add_css_class("note-row-pin")
        pin_icon.add_css_class("dim-label")
        pin_icon.set_visible(False)

        filename_label = Gtk.Label(xalign=0)
        filename_label.add_css_class("caption")
        filename_label.add_css_class("dim-label")
        filename_label.add_css_class("note-row-filename")
        filename_label.set_ellipsize(Pango.EllipsizeMode.END)

        language = Gtk.Label(xalign=0)
        language.add_css_class("caption")
        language.add_css_class("note-row-language")

        modified = Gtk.Label(xalign=1)
        modified.add_css_class("caption")
        modified.add_css_class("dim-label")
        modified.set_halign(Gtk.Align.END)

        header_spacer = Gtk.Box(hexpand=True)
        header_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        header_box.append(title_label)
        header_box.append(pin_icon)
        header_box.append(header_spacer)
        header_box.append(modified)

        body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
        body.add_css_class("note-row-card-content")
        body.append(header_box)
        filename_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        filename_row.append(filename_label)
        filename_row.append(Gtk.Box(hexpand=True))
        filename_row.append(language)
        body.append(filename_row)

        folder_indicator = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        folder_indicator.add_css_class("note-row-folder-indicator")

        card = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        card.add_css_class("note-row-card")
        card.append(folder_indicator)
        card.append(body)

        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        outer.add_css_class("note-row-outer")
        outer.set_margin_start(18)
        outer.set_margin_end(18)
        outer.set_margin_top(2)
        outer.set_margin_bottom(2)
        outer.append(card)
        primary_click = Gtk.GestureClick(button=Gdk.BUTTON_PRIMARY)
        primary_click.connect("released", self._on_row_clicked)
        outer.add_controller(primary_click)
        secondary_click = Gtk.GestureClick(button=Gdk.BUTTON_SECONDARY)
        secondary_click.connect("pressed", self._on_secondary_click_pressed)
        outer.add_controller(secondary_click)
        self.append(outer)

        self._title_label = title_label
        self._filename_label = filename_label
        self._language = language
        self._modified = modified
        self._pin_icon = pin_icon
        self._card = card
        self._folder_indicator = folder_indicator
        self._outer = outer
        self._filename_anchor = Gdk.Rectangle()
        self._suppress_first_menu_prelight = True
        self._menu_popover = self._build_menu_popover()
        self._rename_popover = self._build_rename_popover()
        self.update(note)

    def update(self, note: NoteRecord) -> None:
        self.note_path = note.path
        self._title_label.set_text(note.title)
        self._filename_label.set_text(note.path.name)
        self._modified.set_text(_format_modified(note.modified_at))

    def set_language_label(self, label: str) -> None:
        self._language.set_text(label)

    def set_active(self, active: bool) -> None:
        self._active = active
        if active:
            self._card.add_css_class("active")
        else:
            self._card.remove_css_class("active")

    def set_pinned(self, pinned: bool) -> None:
        self._pinned = pinned
        self._pin_icon.set_visible(pinned)
        if hasattr(self, "_pin_label"):
            self._pin_label.set_text("Unpin" if pinned else "Pin to Top")

    def set_move_targets(self, folders: list[tuple[str, str]]) -> None:
        self._move_targets = folders
        if hasattr(self, "_move_to_folder_button"):
            self._rebuild_move_targets()

    def set_folder_color_token(self, color_token: str) -> None:
        if self._folder_color_class:
            self._folder_indicator.remove_css_class(self._folder_color_class)
        color_class = folder_color_css_class(color_token)
        self._folder_indicator.add_css_class(color_class)
        self._folder_color_class = color_class

    def _build_menu_popover(self) -> Gtk.Popover:
        popover = Gtk.Popover()
        popover.add_css_class("papertrail-popover")
        popover.add_css_class("note-row-menu-popover")
        popover.set_parent(self._title_label)
        popover.set_position(Gtk.PositionType.BOTTOM)
        popover.set_has_arrow(True)
        popover.set_autohide(True)

        rename_button = Gtk.Button()
        rename_button.add_css_class("flat")
        rename_button.add_css_class("modelbutton")
        rename_button.set_can_focus(False)
        rename_button.set_focus_on_click(False)
        rename_button.set_halign(Gtk.Align.FILL)
        rename_button.set_hexpand(True)
        rename_button.connect("clicked", self._on_rename_action_activated)
        rename_label = Gtk.Label(label="Rename", xalign=0)
        rename_label.set_halign(Gtk.Align.START)
        rename_label.set_hexpand(True)
        rename_button.set_child(rename_label)

        pin_button = Gtk.Button()
        pin_button.add_css_class("flat")
        pin_button.add_css_class("modelbutton")
        pin_button.set_can_focus(False)
        pin_button.set_focus_on_click(False)
        pin_button.set_halign(Gtk.Align.FILL)
        pin_button.set_hexpand(True)
        pin_button.connect("clicked", self._on_pin_action_activated)
        pin_label = Gtk.Label(label="Pin to Top", xalign=0)
        pin_label.set_halign(Gtk.Align.START)
        pin_label.set_hexpand(True)
        pin_button.set_child(pin_label)

        open_folder_button = Gtk.Button()
        open_folder_button.add_css_class("flat")
        open_folder_button.add_css_class("modelbutton")
        open_folder_button.set_can_focus(False)
        open_folder_button.set_focus_on_click(False)
        open_folder_button.set_halign(Gtk.Align.FILL)
        open_folder_button.set_hexpand(True)
        open_folder_button.connect("clicked", self._on_open_folder_action_activated)
        open_folder_label = Gtk.Label(label="Open Folder", xalign=0)
        open_folder_label.set_halign(Gtk.Align.START)
        open_folder_label.set_hexpand(True)
        open_folder_button.set_child(open_folder_label)

        move_to_folder_button = Gtk.Button()
        move_to_folder_button.add_css_class("flat")
        move_to_folder_button.add_css_class("modelbutton")
        move_to_folder_button.add_css_class("submenu-button")
        move_to_folder_button.set_can_focus(False)
        move_to_folder_button.set_focus_on_click(False)
        move_to_folder_button.set_halign(Gtk.Align.FILL)
        move_to_folder_button.set_hexpand(True)
        move_to_folder_button.connect("clicked", self._on_move_to_folder_action_activated)
        move_to_folder_label = Gtk.Label(label="Move to Folder", xalign=0)
        move_to_folder_label.add_css_class("submenu-label")
        move_to_folder_label.set_halign(Gtk.Align.START)
        move_to_folder_label.set_hexpand(True)
        move_to_folder_chevron = Gtk.Image.new_from_icon_name("go-next-symbolic")
        move_to_folder_chevron.add_css_class("dim-label")
        move_to_folder_content = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        move_to_folder_content.set_halign(Gtk.Align.FILL)
        move_to_folder_content.set_hexpand(True)
        move_to_folder_content.append(move_to_folder_label)
        move_to_folder_content.append(move_to_folder_chevron)
        move_to_folder_button.set_child(move_to_folder_content)

        move_to_folder_popover = Gtk.Popover()
        move_to_folder_popover.add_css_class("papertrail-popover")
        move_to_folder_popover.add_css_class("note-row-menu-popover")
        move_to_folder_popover.add_css_class("move-target-popover")
        move_to_folder_popover.set_parent(move_to_folder_button)
        move_to_folder_popover.set_position(Gtk.PositionType.RIGHT)
        move_to_folder_popover.set_has_arrow(True)
        move_to_folder_popover.set_autohide(True)

        move_to_folder_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        move_to_folder_box.set_halign(Gtk.Align.FILL)
        move_to_folder_box.set_hexpand(True)
        move_to_folder_popover.set_child(move_to_folder_box)

        delete_button = Gtk.Button()
        delete_button.add_css_class("flat")
        delete_button.add_css_class("modelbutton")
        delete_button.add_css_class("destructive-action")
        delete_button.set_can_focus(False)
        delete_button.set_focus_on_click(False)
        delete_button.set_halign(Gtk.Align.FILL)
        delete_button.set_hexpand(True)
        delete_button.connect("clicked", self._on_delete_action_activated)
        delete_label = Gtk.Label(label="Delete", xalign=0)
        delete_label.set_halign(Gtk.Align.START)
        delete_label.set_hexpand(True)
        delete_button.set_child(delete_label)

        if self._suppress_first_menu_prelight:
            rename_button.set_sensitive(False)
            pin_button.set_sensitive(False)
            open_folder_button.set_sensitive(False)
            move_to_folder_button.set_sensitive(False)
            delete_button.set_sensitive(False)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        content.set_halign(Gtk.Align.FILL)
        content.set_hexpand(True)
        content.append(pin_button)
        content.append(rename_button)
        content.append(open_folder_button)
        content.append(move_to_folder_button)
        content.append(delete_button)
        popover.set_child(content)
        self._menu_buttons = (
            pin_button,
            rename_button,
            open_folder_button,
            move_to_folder_button,
            delete_button,
        )
        self._pin_label = pin_label
        self._move_to_folder_button = move_to_folder_button
        self._move_to_folder_label = move_to_folder_label
        self._move_to_folder_chevron = move_to_folder_chevron
        self._move_to_folder_popover = move_to_folder_popover
        self._move_to_folder_box = move_to_folder_box
        self._rebuild_move_targets()
        return popover

    def _rebuild_move_targets(self) -> None:
        child = self._move_to_folder_box.get_first_child()
        while child is not None:
            next_child = child.get_next_sibling()
            self._move_to_folder_box.remove(child)
            child = next_child

        if not self._move_targets:
            self._move_to_folder_button.set_sensitive(False)
            self._move_to_folder_chevron.set_visible(False)
            return

        self._move_to_folder_button.set_sensitive(not self._suppress_first_menu_prelight)
        self._move_to_folder_chevron.set_visible(True)
        for folder_path, color_token in self._move_targets:
            button = Gtk.Button()
            button.add_css_class("flat")
            button.add_css_class("modelbutton")
            button.set_can_focus(False)
            button.set_focus_on_click(False)
            button.set_halign(Gtk.Align.FILL)
            button.set_hexpand(True)
            button.set_tooltip_text(folder_path)
            button.connect("clicked", self._on_move_target_clicked, folder_path)

            icon = Gtk.Image.new_from_icon_name("folder-symbolic")
            icon.add_css_class("move-target-icon")
            icon.add_css_class(folder_color_css_class(color_token))

            title = Gtk.Label(label=GLib.path_get_basename(folder_path) or folder_path, xalign=0)
            title.set_halign(Gtk.Align.START)
            title.set_hexpand(True)

            badge = Gtk.Label(label=_folder_badge_text(GLib.path_get_basename(folder_path) or folder_path), xalign=0.5)
            badge.add_css_class("caption")
            badge.add_css_class("dim-label")

            row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
            row.append(icon)
            row.append(title)
            row.append(badge)
            button.set_child(row)
            self._move_to_folder_box.append(button)

    def _build_rename_popover(self) -> Gtk.Popover:
        popover = Gtk.Popover()
        popover.add_css_class("papertrail-popover")
        popover.add_css_class("rename-popover")
        popover.set_parent(self._outer)
        popover.set_position(Gtk.PositionType.BOTTOM)

        title = Gtk.Label(label="Rename note", xalign=0)
        title.add_css_class("heading")

        entry = Gtk.Entry()
        entry.set_activates_default(True)
        entry.add_css_class("rename-popover-entry")
        entry.connect("activate", self._submit_rename)

        cancel_button = Gtk.Button(label="Cancel")
        cancel_button.add_css_class("flat")
        cancel_button.connect("clicked", lambda *_args: popover.popdown())

        rename_button = Gtk.Button(label="Rename")
        rename_button.add_css_class("suggested-action")
        rename_button.connect("clicked", self._submit_rename)

        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        actions.set_halign(Gtk.Align.END)
        actions.append(cancel_button)
        actions.append(rename_button)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        content.set_margin_top(12)
        content.set_margin_bottom(12)
        content.set_margin_start(12)
        content.set_margin_end(12)
        content.append(title)
        content.append(entry)
        content.append(actions)

        popover.set_child(content)
        self._rename_entry = entry
        return popover

    def _on_row_clicked(self, *_args) -> None:
        self.emit("activated")

    def _on_secondary_click_pressed(
        self,
        _gesture: Gtk.GestureClick,
        _n_press: int,
        _x: float,
        _y: float,
    ) -> None:
        self._filename_anchor.x = self._outer.get_allocated_width() // 2
        self._filename_anchor.y = self._outer.get_allocated_height() // 2
        self._filename_anchor.width = 1
        self._filename_anchor.height = 1
        self._move_to_folder_popover.popdown()
        self._menu_popover.popdown()
        self._menu_popover.popup()
        if self._suppress_first_menu_prelight:
            GLib.idle_add(self._enable_first_menu_buttons, self._menu_popover)

    def _on_rename_action_activated(self, *_args) -> None:
        if self._menu_popover is not None:
            self._menu_popover.popdown()
        self._rename_entry.set_text(self.note_path.name)
        self._rename_entry.select_region(0, -1)
        self._rename_popover.set_pointing_to(self._filename_anchor)
        self._rename_popover.popup()
        self._rename_entry.grab_focus()

    def _on_pin_action_activated(self, *_args) -> None:
        self._menu_popover.popdown()
        self.emit("pin-toggled", not self._pinned)

    def _on_open_folder_action_activated(self, *_args) -> None:
        self._menu_popover.popdown()
        self.emit("open-folder-requested")

    def _on_move_to_folder_action_activated(self, *_args) -> None:
        if self._move_targets:
            self._move_to_folder_popover.popup()

    def _on_move_target_clicked(self, _button: Gtk.Button, folder_path: str) -> None:
        self._move_to_folder_popover.popdown()
        self._menu_popover.popdown()
        self.emit("move-to-folder-requested", folder_path)

    def _on_delete_action_activated(self, *_args) -> None:
        self._menu_popover.popdown()
        self.emit("delete-requested")

    def _enable_first_menu_buttons(self, popover: Gtk.Popover) -> bool:
        if self._menu_popover is popover:
            for button in self._menu_buttons:
                button.set_sensitive(button is not self._move_to_folder_button or bool(self._move_targets))
            self._suppress_first_menu_prelight = False
        return False

    def _submit_rename(self, *_args) -> None:
        filename = self._rename_entry.get_text().strip()
        if not filename or filename == self.note_path.name:
            self._rename_popover.popdown()
            return
        self._rename_popover.popdown()
        self.emit("rename-submitted", filename)
