from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageCallInfoDataItem")



@_attrs_define
class PageCallInfoDataItem:
    """ 
        Attributes:
            id (str):
            state (str):
            from_uri (str):
            to_uri (str):
            created_at (str):
            media_anchored (bool):
            from_display_name (None | str | Unset):
            to_display_name (None | str | Unset):
            established_at (None | str | Unset):
            ended_at (None | str | Unset):
            duration_seconds (int | None | Unset):
            failure_reason (None | str | Unset):
     """

    id: str
    state: str
    from_uri: str
    to_uri: str
    created_at: str
    media_anchored: bool
    from_display_name: None | str | Unset = UNSET
    to_display_name: None | str | Unset = UNSET
    established_at: None | str | Unset = UNSET
    ended_at: None | str | Unset = UNSET
    duration_seconds: int | None | Unset = UNSET
    failure_reason: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        state = self.state

        from_uri = self.from_uri

        to_uri = self.to_uri

        created_at = self.created_at

        media_anchored = self.media_anchored

        from_display_name: None | str | Unset
        if isinstance(self.from_display_name, Unset):
            from_display_name = UNSET
        else:
            from_display_name = self.from_display_name

        to_display_name: None | str | Unset
        if isinstance(self.to_display_name, Unset):
            to_display_name = UNSET
        else:
            to_display_name = self.to_display_name

        established_at: None | str | Unset
        if isinstance(self.established_at, Unset):
            established_at = UNSET
        else:
            established_at = self.established_at

        ended_at: None | str | Unset
        if isinstance(self.ended_at, Unset):
            ended_at = UNSET
        else:
            ended_at = self.ended_at

        duration_seconds: int | None | Unset
        if isinstance(self.duration_seconds, Unset):
            duration_seconds = UNSET
        else:
            duration_seconds = self.duration_seconds

        failure_reason: None | str | Unset
        if isinstance(self.failure_reason, Unset):
            failure_reason = UNSET
        else:
            failure_reason = self.failure_reason


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "state": state,
            "from_uri": from_uri,
            "to_uri": to_uri,
            "created_at": created_at,
            "media_anchored": media_anchored,
        })
        if from_display_name is not UNSET:
            field_dict["from_display_name"] = from_display_name
        if to_display_name is not UNSET:
            field_dict["to_display_name"] = to_display_name
        if established_at is not UNSET:
            field_dict["established_at"] = established_at
        if ended_at is not UNSET:
            field_dict["ended_at"] = ended_at
        if duration_seconds is not UNSET:
            field_dict["duration_seconds"] = duration_seconds
        if failure_reason is not UNSET:
            field_dict["failure_reason"] = failure_reason

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        state = d.pop("state")

        from_uri = d.pop("from_uri")

        to_uri = d.pop("to_uri")

        created_at = d.pop("created_at")

        media_anchored = d.pop("media_anchored")

        def _parse_from_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        from_display_name = _parse_from_display_name(d.pop("from_display_name", UNSET))


        def _parse_to_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        to_display_name = _parse_to_display_name(d.pop("to_display_name", UNSET))


        def _parse_established_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        established_at = _parse_established_at(d.pop("established_at", UNSET))


        def _parse_ended_at(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        ended_at = _parse_ended_at(d.pop("ended_at", UNSET))


        def _parse_duration_seconds(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        duration_seconds = _parse_duration_seconds(d.pop("duration_seconds", UNSET))


        def _parse_failure_reason(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        failure_reason = _parse_failure_reason(d.pop("failure_reason", UNSET))


        page_call_info_data_item = cls(
            id=id,
            state=state,
            from_uri=from_uri,
            to_uri=to_uri,
            created_at=created_at,
            media_anchored=media_anchored,
            from_display_name=from_display_name,
            to_display_name=to_display_name,
            established_at=established_at,
            ended_at=ended_at,
            duration_seconds=duration_seconds,
            failure_reason=failure_reason,
        )


        page_call_info_data_item.additional_properties = d
        return page_call_info_data_item

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
