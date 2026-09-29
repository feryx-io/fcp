from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.watcher_response import WatcherResponse





T = TypeVar("T", bound="PresenceDetailResponse")



@_attrs_define
class PresenceDetailResponse:
    """ GET /presence/:aor — Get a single presentity with its watchers.
    One presentity with its watchers.

        Attributes:
            aor (str):
            state (str):
            watcher_count (int):
            watchers (list[WatcherResponse]):
            note (None | str | Unset):
     """

    aor: str
    state: str
    watcher_count: int
    watchers: list[WatcherResponse]
    note: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.watcher_response import WatcherResponse # noqa: PLC0415
        aor = self.aor

        state = self.state

        watcher_count = self.watcher_count

        watchers = []
        for watchers_item_data in self.watchers:
            watchers_item = watchers_item_data.to_dict()
            watchers.append(watchers_item)



        note: None | str | Unset
        if isinstance(self.note, Unset):
            note = UNSET
        else:
            note = self.note


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "aor": aor,
            "state": state,
            "watcher_count": watcher_count,
            "watchers": watchers,
        })
        if note is not UNSET:
            field_dict["note"] = note

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.watcher_response import WatcherResponse # noqa: PLC0415
        d = dict(src_dict)
        aor = d.pop("aor")

        state = d.pop("state")

        watcher_count = d.pop("watcher_count")

        watchers = []
        _watchers = d.pop("watchers")
        for watchers_item_data in (_watchers):
            watchers_item = WatcherResponse.from_dict(watchers_item_data)



            watchers.append(watchers_item)


        def _parse_note(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        note = _parse_note(d.pop("note", UNSET))


        presence_detail_response = cls(
            aor=aor,
            state=state,
            watcher_count=watcher_count,
            watchers=watchers,
            note=note,
        )


        presence_detail_response.additional_properties = d
        return presence_detail_response

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
