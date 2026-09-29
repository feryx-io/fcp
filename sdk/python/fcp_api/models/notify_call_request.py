from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="NotifyCallRequest")



@_attrs_define
class NotifyCallRequest:
    """ 
        Attributes:
            to (str): The extension or SIP URI to ring.
            text (str): What to say once it is answered.
            ring_timeout_secs (int | Unset): How long to ring.
            repeat (int | Unset): How often the notice is spoken.
            call_id (None | str | Unset): The call this is about, for the log.
            test (bool | Unset): Whether it is about a test call.
     """

    to: str
    text: str
    ring_timeout_secs: int | Unset = UNSET
    repeat: int | Unset = UNSET
    call_id: None | str | Unset = UNSET
    test: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        to = self.to

        text = self.text

        ring_timeout_secs = self.ring_timeout_secs

        repeat = self.repeat

        call_id: None | str | Unset
        if isinstance(self.call_id, Unset):
            call_id = UNSET
        else:
            call_id = self.call_id

        test = self.test


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "to": to,
            "text": text,
        })
        if ring_timeout_secs is not UNSET:
            field_dict["ring_timeout_secs"] = ring_timeout_secs
        if repeat is not UNSET:
            field_dict["repeat"] = repeat
        if call_id is not UNSET:
            field_dict["call_id"] = call_id
        if test is not UNSET:
            field_dict["test"] = test

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        to = d.pop("to")

        text = d.pop("text")

        ring_timeout_secs = d.pop("ring_timeout_secs", UNSET)

        repeat = d.pop("repeat", UNSET)

        def _parse_call_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        call_id = _parse_call_id(d.pop("call_id", UNSET))


        test = d.pop("test", UNSET)

        notify_call_request = cls(
            to=to,
            text=text,
            ring_timeout_secs=ring_timeout_secs,
            repeat=repeat,
            call_id=call_id,
            test=test,
        )


        notify_call_request.additional_properties = d
        return notify_call_request

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
