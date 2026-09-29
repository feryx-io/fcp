from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageDebugCallInfoDataItem")



@_attrs_define
class PageDebugCallInfoDataItem:
    """ Active calls information.

        Attributes:
            call_id (str):
            state (str):
            media_session_id (None | str | Unset):
            a_leg_dialog_id (None | str | Unset):
            b_leg_dialog_id (None | str | Unset):
     """

    call_id: str
    state: str
    media_session_id: None | str | Unset = UNSET
    a_leg_dialog_id: None | str | Unset = UNSET
    b_leg_dialog_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        call_id = self.call_id

        state = self.state

        media_session_id: None | str | Unset
        if isinstance(self.media_session_id, Unset):
            media_session_id = UNSET
        else:
            media_session_id = self.media_session_id

        a_leg_dialog_id: None | str | Unset
        if isinstance(self.a_leg_dialog_id, Unset):
            a_leg_dialog_id = UNSET
        else:
            a_leg_dialog_id = self.a_leg_dialog_id

        b_leg_dialog_id: None | str | Unset
        if isinstance(self.b_leg_dialog_id, Unset):
            b_leg_dialog_id = UNSET
        else:
            b_leg_dialog_id = self.b_leg_dialog_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "state": state,
        })
        if media_session_id is not UNSET:
            field_dict["media_session_id"] = media_session_id
        if a_leg_dialog_id is not UNSET:
            field_dict["a_leg_dialog_id"] = a_leg_dialog_id
        if b_leg_dialog_id is not UNSET:
            field_dict["b_leg_dialog_id"] = b_leg_dialog_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        call_id = d.pop("call_id")

        state = d.pop("state")

        def _parse_media_session_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        media_session_id = _parse_media_session_id(d.pop("media_session_id", UNSET))


        def _parse_a_leg_dialog_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        a_leg_dialog_id = _parse_a_leg_dialog_id(d.pop("a_leg_dialog_id", UNSET))


        def _parse_b_leg_dialog_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        b_leg_dialog_id = _parse_b_leg_dialog_id(d.pop("b_leg_dialog_id", UNSET))


        page_debug_call_info_data_item = cls(
            call_id=call_id,
            state=state,
            media_session_id=media_session_id,
            a_leg_dialog_id=a_leg_dialog_id,
            b_leg_dialog_id=b_leg_dialog_id,
        )


        page_debug_call_info_data_item.additional_properties = d
        return page_debug_call_info_data_item

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
