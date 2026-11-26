from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PersonPresenceResponse")



@_attrs_define
class PersonPresenceResponse:
    """ One person's presence as the tenant's people see it
    (`docs/PRESENCE.md`): what they chose, a call, DND, idle and whether any
    of their phones is registered, worked out by the admin.

        Attributes:
            user_id (str):
            username (str):
            idle (bool): Away because idle, not because they said so.
            display_name (None | str | Unset):
            extension (None | str | Unset):
            state (None | str | Unset): `available`, `away`, `busy`, `dnd`, `on_call`, `offline`; null when
                this admin does not work presence out.
            label (None | str | Unset): Their own status's label ("WFH").
            note (None | str | Unset):
     """

    user_id: str
    username: str
    idle: bool
    display_name: None | str | Unset = UNSET
    extension: None | str | Unset = UNSET
    state: None | str | Unset = UNSET
    label: None | str | Unset = UNSET
    note: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        user_id = self.user_id

        username = self.username

        idle = self.idle

        display_name: None | str | Unset
        if isinstance(self.display_name, Unset):
            display_name = UNSET
        else:
            display_name = self.display_name

        extension: None | str | Unset
        if isinstance(self.extension, Unset):
            extension = UNSET
        else:
            extension = self.extension

        state: None | str | Unset
        if isinstance(self.state, Unset):
            state = UNSET
        else:
            state = self.state

        label: None | str | Unset
        if isinstance(self.label, Unset):
            label = UNSET
        else:
            label = self.label

        note: None | str | Unset
        if isinstance(self.note, Unset):
            note = UNSET
        else:
            note = self.note


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "user_id": user_id,
            "username": username,
            "idle": idle,
        })
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if extension is not UNSET:
            field_dict["extension"] = extension
        if state is not UNSET:
            field_dict["state"] = state
        if label is not UNSET:
            field_dict["label"] = label
        if note is not UNSET:
            field_dict["note"] = note

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        user_id = d.pop("user_id")

        username = d.pop("username")

        idle = d.pop("idle")

        def _parse_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        display_name = _parse_display_name(d.pop("display_name", UNSET))


        def _parse_extension(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        extension = _parse_extension(d.pop("extension", UNSET))


        def _parse_state(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        state = _parse_state(d.pop("state", UNSET))


        def _parse_label(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        label = _parse_label(d.pop("label", UNSET))


        def _parse_note(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        note = _parse_note(d.pop("note", UNSET))


        person_presence_response = cls(
            user_id=user_id,
            username=username,
            idle=idle,
            display_name=display_name,
            extension=extension,
            state=state,
            label=label,
            note=note,
        )


        person_presence_response.additional_properties = d
        return person_presence_response

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
