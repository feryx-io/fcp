from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PresenceResponse")



@_attrs_define
class PresenceResponse:
    """ Presence info response for a single presentity.

        Attributes:
            aor (str):
            state (str):
            watcher_count (int):
            note (None | str | Unset):
            extension (None | str | Unset): Extension number (e.g., "1234") if this AOR is an extension or device belonging
                to one.
            display_name (None | str | Unset): Human-readable display name (user or device name).
     """

    aor: str
    state: str
    watcher_count: int
    note: None | str | Unset = UNSET
    extension: None | str | Unset = UNSET
    display_name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        aor = self.aor

        state = self.state

        watcher_count = self.watcher_count

        note: None | str | Unset
        if isinstance(self.note, Unset):
            note = UNSET
        else:
            note = self.note

        extension: None | str | Unset
        if isinstance(self.extension, Unset):
            extension = UNSET
        else:
            extension = self.extension

        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "aor": aor,
            "state": state,
            "watcher_count": watcher_count,
        })
        if note is not UNSET:
            field_dict["note"] = note
        if extension is not UNSET:
            field_dict["extension"] = extension
        if display_name is not UNSET:
            field_dict["display_name"] = display_name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        aor = d.pop("aor")

        state = d.pop("state")

        watcher_count = d.pop("watcher_count")

        def _parse_note(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        note = _parse_note(d.pop("note", UNSET))


        def _parse_extension(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        extension = _parse_extension(d.pop("extension", UNSET))


        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        presence_response = cls(
            aor=aor,
            state=state,
            watcher_count=watcher_count,
            note=note,
            extension=extension,
            display_name=display_name,
        )


        presence_response.additional_properties = d
        return presence_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
