"""Controlled tabs with a native swipeable, snapping page viewport."""
from .widgets import Widget, Column, Row, Button, Text, invoke


class _Page(Column):
    def _tree_attrs(self):
        return {"role": "tabpanel"}

    def _tag_attrs(self):
        return {"style": "scroll-snap-align:start;scroll-snap-stop:always", "role": "tabpanel"}


class _TabBar(Row):
    def _tag_attrs(self):
        return {"role": "tablist"}

    def _tree_attrs(self):
        return {"role": "tablist"}


class _TabButton(Button):
    def _tag_attrs(self):
        return {**super()._tag_attrs(), "role": "tab", "aria-selected": self.props["aria_selected"]}

    def _tree_attrs(self):
        return {**super()._tree_attrs(), "role": "tab", "selected": self.props["aria_selected"] == "true"}


class _Pager(Row):
    def __init__(self, pages, index, on_change, key):
        super().__init__("flex-1 min-h-0 overflow-x-auto", *[
            _Page("w-full h-full shrink-0 min-w-0 min-h-0", page) for page in pages
        ], key=key, on_change=on_change)
        self.index, self.count = index, len(pages)

    def _payload_parser(self, event):
        return int if event == "change" else super()._payload_parser(event)

    def _tree_attrs(self):
        return {**super()._tree_attrs(), "pager_index": self.index, "pager_count": self.count}

    def _tag_attrs(self):
        return {**super()._tag_attrs(), "data-zkite-pager": self.index,
                "style": "scroll-snap-type:x mandatory;scrollbar-width:none"}



class Tabs(Column):
    """Tabs(classes, *pages, labels=[...], index=0, on_change=..., key=...).

    Keep index in a store. Give each Tabs a stable, unique key. All pages stay
    mounted, preserving child state and vertical scroll positions across swipes.
    """
    def __init__(self, class_name="", *pages, labels, index=0, on_change, key=None):
        if not pages or len(labels) != len(pages):
            raise ValueError("Tabs needs one label for each page and at least one page")
        if isinstance(index, bool) or not isinstance(index, int) or not 0 <= index < len(pages):
            raise ValueError("Tabs index must be an integer within the page range")
        def select(value):
            if 0 <= value < len(pages) and value != index:
                invoke(on_change, value)
        header = _TabBar("shrink-0 bg-white", *[
            _TabButton("flex-1 py-4 items-center border-b-2 " + (
                "border-orange-600" if i == index else "border-white"),
                Text("text-sm font-semibold " + ("text-orange-700" if i == index else "text-slate-500"), label),
                on_tap=lambda _payload=None, i=i: select(i), role="tab", aria_selected=str(i == index).lower())
            for i, label in enumerate(labels)
        ], role="tablist")
        super().__init__("min-h-0 " + class_name, header, _Pager(pages, index, select, f"{key}:pages" if key else None), key=key)
