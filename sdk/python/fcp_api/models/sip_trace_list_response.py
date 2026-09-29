from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.sip_trace_response import SipTraceResponse





T = TypeVar("T", bound="SipTraceListResponse")



@_attrs_define
class SipTraceListResponse:
    """ Response for SIP trace messages for a call.

        Attributes:
            call_id (str):
            messages (list[SipTraceResponse]):
     """

    call_id: str
    messages: list[SipTraceResponse]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.sip_trace_response import SipTraceResponse # noqa: PLC0415
        call_id = self.call_id

        messages = []
        for messages_item_data in self.messages:
            messages_item = messages_item_data.to_dict()
            messages.append(messages_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "messages": messages,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.sip_trace_response import SipTraceResponse # noqa: PLC0415
        d = dict(src_dict)
        call_id = d.pop("call_id")

        messages = []
        _messages = d.pop("messages")
        for messages_item_data in (_messages):
            messages_item = SipTraceResponse.from_dict(messages_item_data)



            messages.append(messages_item)


        sip_trace_list_response = cls(
            call_id=call_id,
            messages=messages,
        )


        sip_trace_list_response.additional_properties = d
        return sip_trace_list_response

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
