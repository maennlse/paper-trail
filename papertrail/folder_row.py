# SPDX-License-Identifier: GPL-3.0-or-later

from __future__ import annotations

from pathlib import Path

from gi.repository import GLib, GObject, Gdk, Gtk

from .folder_colors import folder_color_css_class


class FolderRow(Gtk.Button):
    __gsignals__ = {
        "edit-requested": (GObject.SignalFlags.RUN_FIRST, None, ()),
        "close-requested": (GObject.SignalFlags.RUN_FIRST, None, ()),
        "reorder-requested": (GObject.SignalFlags.RUN_FIRST, None, (str, bool)),
    }

    def __init__(
        self,
        path: Path,
        color_token: str,
        *,
        title: str | None = None,
        badge_text: str | None = None,
        tooltip_text: str | None = None,
        icon_name: str = "folder-symbolic",
        menu_enabled: bool = True,
        drag_enabled: bool = True,
    ) -> None:
        super().__init__()
        self.folder_path = path
        self._color_class = ""
        self._title_override = title
        self._badge_text_override = badge_text
        self._tooltip_override = tooltip_text
        self._menu_enabled = menu_enabled
        self.add_css_class("flat")
        self.add_css_class("folder-row")
        self.set_halign(Gtk.Align.FILL)
        self.set_hexpand(True)
        self.set_tooltip_text(tooltip_text or str(path))

        icon = Gtk.Image.new_from_icon_name(icon_name)
        icon.add_css_class("folder-row-icon")
        icon.set_pixel_size(34)

        initials = Gtk.Label(xalign=0.5)
        initials.add_css_class("folder-row-initials")
        initials.set_halign(Gtk.Align.CENTER)
        initials.set_valign(Gtk.Align.CENTER)

        icon_overlay = Gtk.Overlay()
        icon_overlay.set_child(icon)
        icon_overlay.add_overlay(initials)

        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=0)
        card.add_css_class("folder-row-card")
        card.append(icon_overlay)

        self.set_child(card)
        self._icon = icon
        self._initials = initials
        self._card = card
        self._suppress_first_menu_prelight = True
        self._menu_popover = self._build_menu_popover() if menu_enabled else None
        self._menu_anchor = Gdk.Rectangle()
        if menu_enabled:
            right_click = Gtk.GestureClick(button=Gdk.BUTTON_SECONDARY)
            right_click.connect("released", self._on_secondary_click_released)
            self.add_controller(right_click)
        if drag_enabled:
            self._setup_drag_and_drop()
        self._apply_color_class(color_token)
        self.update(path)

    def update(self, path: Path) -> None:
        self.folder_path = path
        title = self._title_override or path.name or str(path)
        self._initials.set_text(self._badge_text_override or self._folder_badge_text(title))
        self.set_tooltip_text(self._tooltip_override or str(path))

    def set_color_token(self, color_token: str) -> None:
        self._apply_color_class(color_token)

    def _apply_color_class(self, color_token: str) -> None:
        if self._color_class:
            self._icon.remove_css_class(self._color_class)
            self._initials.remove_css_class(self._color_class)
        new_class = folder_color_css_class(color_token)
        self._icon.add_css_class(new_class)
        self._initials.add_css_class(new_class)
        self._color_class = new_class

    def _folder_badge_text(self, title: str) -> str:
        words = [part for part in title.replace("_", " ").replace("-", " ").split() if part]
        if len(words) >= 2:
            return (words[0][0] + words[1][0]).upper()[:2]
        compact = "".join(ch for ch in title if ch.isalnum())
        return compact[:2].upper() or "F"

    def _build_menu_popover(self) -> Gtk.Popover:
        popover = Gtk.Popover()
        popover.add_css_class("papertrail-popover")
        popover.add_css_class("note-row-menu-popover")
        popover.set_parent(self._card)
        popover.set_position(Gtk.PositionType.RIGHT)
        popover.set_has_arrow(True)
        popover.set_offset(0, 0)
        popover.set_autohide(True)

        edit_button = Gtk.Button()
        edit_button.add_css_class("flat")
        edit_button.add_css_class("modelbutton")
        edit_button.set_can_focus(False)
        edit_button.set_focus_on_click(False)
        edit_button.set_halign(Gtk.Align.FILL)
        edit_button.set_hexpand(True)
        edit_button.connect("clicked", self._on_edit_clicked)
        edit_label = Gtk.Label(label="Edit", xalign=0)
        edit_label.set_halign(Gtk.Align.START)
        edit_label.set_hexpand(True)
        edit_button.set_child(edit_label)

        close_button = Gtk.Button()
        close_button.add_css_class("flat")
        close_button.add_css_class("modelbutton")
        close_button.set_can_focus(False)
        close_button.set_focus_on_click(False)
        close_button.set_halign(Gtk.Align.FILL)
        close_button.set_hexpand(True)
        close_button.connect("clicked", self._on_close_clicked)
        close_label = Gtk.Label(label="Close", xalign=0)
        close_label.set_halign(Gtk.Align.START)
        close_label.set_hexpand(True)
        close_button.set_child(close_label)

        if self._suppress_first_menu_prelight:
            edit_button.set_sensitive(False)
            close_button.set_sensitive(False)

        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        content.set_halign(Gtk.Align.FILL)
        content.set_hexpand(True)
        content.append(edit_button)
        content.append(close_button)
        popover.set_child(content)
        self._menu_buttons = (edit_button, close_button)
        return popover

    def _setup_drag_and_drop(self) -> None:
        drag_source = Gtk.DragSource()
        drag_source.set_actions(Gdk.DragAction.MOVE)
        drag_source.connect("prepare", self._on_drag_prepare)
        drag_source.connect("drag-begin", self._on_drag_begin)
        drag_source.connect("drag-end", self._on_drag_end)
        self.add_controller(drag_source)

        drop_target = Gtk.DropTarget.new(GObject.TYPE_STRING, Gdk.DragAction.MOVE)
        drop_target.connect("motion", self._on_drop_motion)
        drop_target.connect("leave", self._on_drop_leave)
        drop_target.connect("drop", self._on_drop)
        self.add_controller(drop_target)

    def _set_drop_indicator(self, before: bool | None) -> None:
        self.remove_css_class("drop-before")
        self.remove_css_class("drop-after")
        if before is True:
            self.add_css_class("drop-before")
        elif before is False:
            self.add_css_class("drop-after")

    def _on_drag_prepare(
        self,
        _source: Gtk.DragSource,
        _x: float,
        _y: float,
    ) -> Gdk.ContentProvider:
        value = GObject.Value()
        value.init(GObject.TYPE_STRING)
        value.set_string(str(self.folder_path))
        return Gdk.ContentProvider.new_for_value(value)

    def _on_drag_begin(self, _source: Gtk.DragSource, _drag: Gdk.Drag) -> None:
        self.add_css_class("dragging")

    def _on_drag_end(self, _source: Gtk.DragSource, _drag: Gdk.Drag, _delete_data: bool) -> None:
        self.remove_css_class("dragging")
        self._set_drop_indicator(None)

    def _on_drop_motion(self, _target: Gtk.DropTarget, x: float, y: float) -> Gdk.DragAction:
        self._set_drop_indicator(y < (self.get_allocated_height() / 2))
        return Gdk.DragAction.MOVE

    def _on_drop_leave(self, _target: Gtk.DropTarget) -> None:
        self._set_drop_indicator(None)

    def _on_drop(self, _target: Gtk.DropTarget, value: str, _x: float, y: float) -> bool:
        self._set_drop_indicator(None)
        if not value or value == str(self.folder_path):
            return False
        self.emit("reorder-requested", value, y < (self.get_allocated_height() / 2))
        return True

    def _on_secondary_click_released(
        self,
        _gesture: Gtk.GestureClick,
        _n_press: int,
        _x: float,
        _y: float,
    ) -> None:
        if self._menu_popover is None:
            return
        self._menu_anchor.x = self._icon.get_allocated_width()
        self._menu_anchor.y = self._icon.get_allocated_height() // 2
        self._menu_anchor.width = 1
        self._menu_anchor.height = 1
        self._menu_popover.set_pointing_to(self._menu_anchor)
        self._menu_popover.popup()
        if self._suppress_first_menu_prelight:
            GLib.idle_add(self._enable_first_menu_buttons, self._menu_popover)

    def _on_edit_clicked(self, *_args) -> None:
        self._menu_popover.popdown()
        self.emit("edit-requested")

    def _on_close_clicked(self, *_args) -> None:
        self._menu_popover.popdown()
        self.emit("close-requested")

    def _enable_first_menu_buttons(self, popover: Gtk.Popover) -> bool:
        if self._menu_popover is popover:
            for button in self._menu_buttons:
                button.set_sensitive(True)
            self._suppress_first_menu_prelight = False
        return False

    def set_active(self, active: bool) -> None:
        if active:
            self.add_css_class("active")
            self._card.add_css_class("active")
        else:
            self.remove_css_class("active")
            self._card.remove_css_class("active")
