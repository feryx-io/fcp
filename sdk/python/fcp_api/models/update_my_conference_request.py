from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateMyConferenceRequest")



@_attrs_define
class UpdateMyConferenceRequest:
    """ What an owner may change on their meeting; anything else is refused
    (an administrator's, on `/conferences/meetings/{id}`). `null` clears a
    PIN or the welcome.

        Attributes:
            name (None | str | Unset):
            guest_pin (None | str | Unset):
            host_pin (None | str | Unset):
            require_guest_pin (bool | None | Unset):
            max_participants (int | None | Unset):
            wait_for_moderator (bool | None | Unset):
            auto_record (bool | None | Unset):
            default_locked (bool | None | Unset):
            welcome_tts (None | str | Unset):
            kind (None | str | Unset): `meeting` or `webinar`.
            panelist_pin (None | str | Unset):
            panelists (list[str] | None | Unset): A webinar's panelists, users of the tenant by username or id.
     """

    name: None | str | Unset = UNSET
    guest_pin: None | str | Unset = UNSET
    host_pin: None | str | Unset = UNSET
    require_guest_pin: bool | None | Unset = UNSET
    max_participants: int | None | Unset = UNSET
    wait_for_moderator: bool | None | Unset = UNSET
    auto_record: bool | None | Unset = UNSET
    default_locked: bool | None | Unset = UNSET
    welcome_tts: None | str | Unset = UNSET
    kind: None | str | Unset = UNSET
    panelist_pin: None | str | Unset = UNSET
    panelists: list[str] | None | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        guest_pin: None | str | Unset
        if isinstance(self.guest_pin, Unset):
            guest_pin = UNSET
        else:
            guest_pin = self.guest_pin

        host_pin: None | str | Unset
        if isinstance(self.host_pin, Unset):
            host_pin = UNSET
        else:
            host_pin = self.host_pin

        require_guest_pin: bool | None | Unset
        if isinstance(self.require_guest_pin, Unset):
            require_guest_pin = UNSET
        else:
            require_guest_pin = self.require_guest_pin

        max_participants: int | None | Unset
        if isinstance(self.max_participants, Unset):
            max_participants = UNSET
        else:
            max_participants = self.max_participants

        wait_for_moderator: bool | None | Unset
        if isinstance(self.wait_for_moderator, Unset):
            wait_for_moderator = UNSET
        else:
            wait_for_moderator = self.wait_for_moderator

        auto_record: bool | None | Unset
        if isinstance(self.auto_record, Unset):
            auto_record = UNSET
        else:
            auto_record = self.auto_record

        default_locked: bool | None | Unset
        if isinstance(self.default_locked, Unset):
            default_locked = UNSET
        else:
            default_locked = self.default_locked

        welcome_tts: None | str | Unset
        if isinstance(self.welcome_tts, Unset):
            welcome_tts = UNSET
        else:
            welcome_tts = self.welcome_tts

        kind: None | str | Unset
        if isinstance(self.kind, Unset):
            kind = UNSET
        else:
            kind = self.kind

        panelist_pin: None | str | Unset
        if isinstance(self.panelist_pin, Unset):
            panelist_pin = UNSET
        else:
            panelist_pin = self.panelist_pin

        panelists: list[str] | None | Unset
        if isinstance(self.panelists, Unset):
            panelists = UNSET
        elif isinstance(self.panelists, list):
            panelists = self.panelists


        else:
            panelists = self.panelists


        field_dict: dict[str, Any] = {}

        field_dict.update({
        })
        if name is not UNSET:
            field_dict["name"] = name
        if guest_pin is not UNSET:
            field_dict["guest_pin"] = guest_pin
        if host_pin is not UNSET:
            field_dict["host_pin"] = host_pin
        if require_guest_pin is not UNSET:
            field_dict["require_guest_pin"] = require_guest_pin
        if max_participants is not UNSET:
            field_dict["max_participants"] = max_participants
        if wait_for_moderator is not UNSET:
            field_dict["wait_for_moderator"] = wait_for_moderator
        if auto_record is not UNSET:
            field_dict["auto_record"] = auto_record
        if default_locked is not UNSET:
            field_dict["default_locked"] = default_locked
        if welcome_tts is not UNSET:
            field_dict["welcome_tts"] = welcome_tts
        if kind is not UNSET:
            field_dict["kind"] = kind
        if panelist_pin is not UNSET:
            field_dict["panelist_pin"] = panelist_pin
        if panelists is not UNSET:
            field_dict["panelists"] = panelists

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_guest_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        guest_pin = _parse_guest_pin(d.pop("guest_pin", UNSET))


        def _parse_host_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        host_pin = _parse_host_pin(d.pop("host_pin", UNSET))


        def _parse_require_guest_pin(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        require_guest_pin = _parse_require_guest_pin(d.pop("require_guest_pin", UNSET))


        def _parse_max_participants(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_participants = _parse_max_participants(d.pop("max_participants", UNSET))


        def _parse_wait_for_moderator(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        wait_for_moderator = _parse_wait_for_moderator(d.pop("wait_for_moderator", UNSET))


        def _parse_auto_record(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        auto_record = _parse_auto_record(d.pop("auto_record", UNSET))


        def _parse_default_locked(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        default_locked = _parse_default_locked(d.pop("default_locked", UNSET))


        def _parse_welcome_tts(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        welcome_tts = _parse_welcome_tts(d.pop("welcome_tts", UNSET))


        def _parse_kind(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        kind = _parse_kind(d.pop("kind", UNSET))


        def _parse_panelist_pin(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        panelist_pin = _parse_panelist_pin(d.pop("panelist_pin", UNSET))


        def _parse_panelists(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                panelists_type_0 = cast(list[str], data)

                return panelists_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        panelists = _parse_panelists(d.pop("panelists", UNSET))


        update_my_conference_request = cls(
            name=name,
            guest_pin=guest_pin,
            host_pin=host_pin,
            require_guest_pin=require_guest_pin,
            max_participants=max_participants,
            wait_for_moderator=wait_for_moderator,
            auto_record=auto_record,
            default_locked=default_locked,
            welcome_tts=welcome_tts,
            kind=kind,
            panelist_pin=panelist_pin,
            panelists=panelists,
        )

        return update_my_conference_request

